// Software Catalog rules shared by the mock and the pages, following routes/portal_model.rs:
// entities inferred from spans' peer attributes, entity definition (apiVersion v3) validation, and
// the merge of telemetry, service metadata and definitions into the catalog's entities.
import type { ServiceContact } from '../scorecards/types';
import type { EntityContact, EntityDefinition, EntityLink, PortalEntity, PortalEntityKind } from './types';

export const DEFINITION_KINDS = ['service', 'datastore', 'queue', 'api', 'system', 'frontend'] as const;
export const LINK_TYPES = ['doc', 'repo', 'runbook', 'dashboard', 'other'] as const;
export const CONTACT_TYPES = ['email', 'slack', 'microsoft-teams', 'link'] as const;
const MAX_NAME = 100;
const MAX_TEXT = 200;
const MAX_DESCRIPTION = 2000;
const MAX_ITEMS = 50;

// ================================================================ inferred entities

/** Outbound (and messaging consumer) spans of one service, grouped by their peer attributes. */
export interface PeerCall {
  service: string;
  dbSystem?: string;
  /** `db.instance`, else `db.namespace` / `db.name`. */
  dbName?: string;
  serverAddress?: string;
  peerService?: string;
  messagingSystem?: string;
  destination?: string;
  hits: number;
  errors: number;
  p95Ms: number;
}

export type InferredKind = 'datastore' | 'queue' | 'external_provider';

export interface InferredEntity {
  kind: InferredKind;
  name: string;
  technology: string | null;
  callers: string[];
  requests: number;
  errors: number;
  p95Ms: number;
}

const firstNamed = (candidates: (string | undefined)[]) => candidates.map((c) => (c ?? '').trim()).find((c) => c && !/^\d+$/.test(c));

/** What a call reaches, or null for calls between instrumented services. */
export function inferPeer(call: PeerCall, instrumented: Set<string>): { kind: InferredKind; name: string; technology: string | null } | null {
  if (call.dbSystem?.trim()) {
    const name = firstNamed([call.dbName, call.peerService, call.serverAddress, call.dbSystem]);
    return name ? { kind: 'datastore', name: name.toLowerCase(), technology: call.dbSystem.trim().toLowerCase() } : null;
  }
  if (call.messagingSystem?.trim()) {
    const name = firstNamed([call.destination, call.peerService, call.serverAddress, call.messagingSystem]);
    return name ? { kind: 'queue', name: name.toLowerCase(), technology: call.messagingSystem.trim().toLowerCase() } : null;
  }
  const peer = call.peerService?.trim();
  if (peer && !instrumented.has(peer)) return { kind: 'external_provider', name: peer.toLowerCase(), technology: null };
  return null;
}

export function inferEntities(calls: PeerCall[], instrumented: Set<string>): InferredEntity[] {
  const byKey = new Map<string, InferredEntity>();
  for (const c of calls) {
    const p = inferPeer(c, instrumented);
    if (!p) continue;
    const key = `${p.kind}:${p.name}`;
    let e = byKey.get(key);
    if (!e) byKey.set(key, (e = { ...p, callers: [], requests: 0, errors: 0, p95Ms: 0 }));
    if (c.service && !e.callers.includes(c.service)) e.callers.push(c.service);
    e.requests += c.hits;
    e.errors += c.errors;
    e.p95Ms = Math.max(e.p95Ms, Number.isFinite(c.p95Ms) ? c.p95Ms : 0);
  }
  return [...byKey.values()].map((e) => ({ ...e, callers: e.callers.sort() })).sort((a, b) => (a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind.localeCompare(b.kind)));
}

// ================================================================ entity definitions

const fail = (message: string): never => {
  throw new Error(message);
};

function text(v: unknown, what: string, max: number): string | null {
  if (v === null || v === undefined) return null;
  const s = String(v).trim();
  if (!s) return null;
  if ([...s].length > max) fail(`${what} has at most ${max} characters`);
  return s;
}

function entityName(name: unknown, what: string): string {
  const n = String(name ?? '')
    .trim()
    .toLowerCase();
  if (!n) fail(`${what} is required`);
  if (n.length > MAX_NAME) fail(`${what} has at most ${MAX_NAME} characters`);
  if (!/^[a-z0-9][a-z0-9_.\/-]*$/.test(n)) fail(`${what} '${n}' may only contain letters, digits, '-', '_', '.' and '/', starting with a letter or digit`);
  return n;
}

export const isHttpUrl = (url: string) => /^https?:\/\/\S+$/i.test(url.trim()) && url.trim().length <= 2048;

function reference(r: unknown, defaultKind: string, what: string): string {
  const s = String(r ?? '')
    .trim()
    .toLowerCase();
  const i = s.indexOf(':');
  const [kind, name] = i >= 0 ? [s.slice(0, i), s.slice(i + 1)] : [defaultKind, s];
  if (!(DEFINITION_KINDS as readonly string[]).includes(kind)) fail(`${what} '${s}' has an unknown kind; use one of ${DEFINITION_KINDS.join(', ')}`);
  return `${kind}:${entityName(name, what)}`;
}

function list(v: unknown, what: string): unknown[] {
  if (v === undefined || v === null) return [];
  if (!Array.isArray(v)) fail(`${what} must be a list`);
  if ((v as unknown[]).length > MAX_ITEMS) fail(`${what} has at most ${MAX_ITEMS} entries`);
  return v as unknown[];
}

const uniq = <T>(xs: T[]) => xs.filter((x, i) => xs.indexOf(x) === i);

/** Checks and normalizes a parsed document, like the server. */
export function validateDefinition(doc: unknown): EntityDefinition {
  if (!doc || typeof doc !== 'object' || Array.isArray(doc)) fail('A document must be an object with apiVersion, kind and metadata');
  const d = doc as Record<string, unknown>;
  for (const key of ['apiVersion', 'kind', 'metadata']) if (!(key in d)) fail(`${key} is required`);
  const apiVersion = String(d.apiVersion).trim();
  if (apiVersion !== 'v3') fail(`apiVersion '${apiVersion}' isn't supported; use v3`);
  const kind = String(d.kind).trim().toLowerCase();
  if (!(DEFINITION_KINDS as readonly string[]).includes(kind)) fail(`kind '${kind}' isn't supported; use one of ${DEFINITION_KINDS.join(', ')}`);
  const m = (d.metadata ?? {}) as Record<string, unknown>;
  const s = (d.spec ?? {}) as Record<string, unknown>;
  const tags = uniq(
    list(m.tags, 'metadata.tags')
      .map((t) => text(t, 'metadata.tags', MAX_TEXT))
      .filter((t): t is string => !!t),
  );
  const spaced = tags.find((t) => /\s/.test(t));
  if (spaced) fail(`Tag '${spaced}' contains whitespace`);
  const links: EntityLink[] = list(m.links, 'metadata.links').map((raw) => {
    const l = raw as Record<string, unknown>;
    const type = String(l.type ?? '')
      .trim()
      .toLowerCase();
    if (!(LINK_TYPES as readonly string[]).includes(type)) fail(`Link type '${type}' isn't supported; use one of ${LINK_TYPES.join(', ')}`);
    const url = String(l.url ?? '').trim();
    if (!isHttpUrl(url)) fail('A link URL must be an http(s) URL');
    const provider = text(l.provider, 'A link provider', MAX_TEXT);
    return { name: text(l.name, 'A link name', MAX_TEXT) ?? url, type, url, ...(provider ? { provider } : {}) };
  });
  const contacts: EntityContact[] = list(m.contacts, 'metadata.contacts').map((raw) => {
    const c = raw as Record<string, unknown>;
    const type = String(c.type ?? '')
      .trim()
      .toLowerCase();
    const contact = String(c.contact ?? '').trim();
    if (!(CONTACT_TYPES as readonly string[]).includes(type)) fail(`Contact type '${type}' isn't supported; use one of ${CONTACT_TYPES.join(', ')}`);
    const ok = type === 'email' ? /^\S+@\S+$/.test(contact) : type === 'slack' ? !!contact && contact.length <= MAX_TEXT : isHttpUrl(contact);
    if (!ok) fail(`'${contact}' isn't a valid ${type} contact`);
    const name = text(c.name, 'A contact name', MAX_TEXT);
    return { ...(name ? { name } : {}), type, contact };
  });
  const owner = text(m.owner, 'metadata.owner', MAX_TEXT)?.replace(/^@+/, '').toLowerCase() || null;
  const refs = (v: unknown, defaultKind: string, what: string) => uniq(list(v, what).map((r) => reference(r, defaultKind, what)));
  const tier = s.tier === undefined || s.tier === null ? null : text(String(s.tier), 'spec.tier', MAX_TEXT);
  const out: EntityDefinition = {
    apiVersion: 'v3',
    kind,
    metadata: {
      name: entityName(m.name, 'metadata.name'),
      displayName: text(m.displayName, 'metadata.displayName', MAX_TEXT),
      owner,
      description: text(m.description, 'metadata.description', MAX_DESCRIPTION),
      tags,
      links,
      contacts,
    },
    spec: {
      lifecycle: text(s.lifecycle, 'spec.lifecycle', MAX_TEXT)?.toLowerCase() ?? null,
      tier,
      type: text(s.type, 'spec.type', MAX_TEXT)?.toLowerCase() ?? null,
      languages: uniq(
        list(s.languages, 'spec.languages')
          .map((l) => text(l, 'spec.languages', MAX_TEXT)?.toLowerCase())
          .filter((l): l is string => !!l),
      ),
      dependsOn: refs(s.dependsOn, 'service', 'spec.dependsOn'),
      componentOf: refs(s.componentOf, 'system', 'spec.componentOf'),
      components: refs(s.components, 'service', 'spec.components'),
      implementedBy: refs(s.implementedBy, 'service', 'spec.implementedBy'),
    },
  };
  for (const key of ['integrations', 'datadog', 'extensions'] as const) if (d[key] && typeof d[key] === 'object') out[key] = d[key] as Record<string, never>;
  return out;
}

/**
 * JSON documents (an object or an array). The server also reads YAML; the in-browser mock has no
 * YAML parser, so it asks for JSON.
 */
export function parseJsonDocuments(content: string): EntityDefinition[] {
  const trimmed = content.trim();
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) fail('In the demo, paste JSON documents (the server also reads YAML)');
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch (e) {
    fail(`Invalid JSON: ${(e as Error).message}`);
  }
  const docs = Array.isArray(parsed) ? parsed : [parsed];
  if (!docs.length) fail('No entity definition found');
  if (docs.length > 200) fail('Import at most 200 entities at a time');
  const seen = new Set<string>();
  return docs.map((doc, i) => {
    let def: EntityDefinition;
    try {
      def = validateDefinition(doc);
    } catch (e) {
      return fail(`Document ${i + 1}: ${(e as Error).message}`);
    }
    const key = `${def.kind}:${def.metadata.name}`;
    if (seen.has(key)) fail(`Document ${i + 1}: ${key} is defined twice`);
    seen.add(key);
    return def;
  });
}

// ================================================================ merge

export interface ApmServiceFact {
  name: string;
  web: boolean;
  language?: string | null;
  version?: string | null;
  requests: number;
  reqPerSec: number;
  errorRate: number;
  p95Ms: number;
  lastDeploy?: number | null;
}

export interface EndpointFact {
  service: string;
  resource: string;
  requests: number;
  reqPerSec: number;
  errorRate: number;
  p95Ms: number;
}

export interface ServiceMetaFact {
  team: string;
  contacts: ServiceContact[];
  repoUrl: string;
  docsUrl: string;
}

export interface ReliabilityFact {
  name: string;
  texts: string[];
  monitors?: string[];
  alerting?: boolean;
}

export interface CiRepoFact {
  name: string;
  url: string;
  runs: number;
  failures: number;
  lastRun: number;
}

export interface CatalogFacts {
  services: ApmServiceFact[];
  logServices: string[];
  metricServices: string[];
  peers: InferredEntity[];
  endpoints: EndpointFact[];
  errorTypes: Record<string, number>;
  definitions: { id: string; definition: EntityDefinition }[];
  metadata: Map<string, ServiceMetaFact>;
  slos: ReliabilityFact[];
  monitors: ReliabilityFact[];
  incidents: { services: string[]; open: boolean; inRange: boolean }[];
  repos: CiRepoFact[];
  /** Matches a service named in a monitor or SLO text (scorecards' rule). */
  references: (text: string, service: string) => boolean;
}

/** `https://git.example.com/example-org/order-service.git` → `example-org/order-service`. */
export function repoPath(urlOrName: string): string {
  let s = urlOrName.trim().replace(/\/+$/, '');
  if (s.includes('://')) s = s.split('://')[1].split('/').slice(1).join('/');
  else if (s.includes(':')) s = s.slice(s.indexOf(':') + 1);
  return s
    .replace(/\.git$/, '')
    .replace(/^\/+|\/+$/g, '')
    .toLowerCase();
}

const KIND_ORDER: PortalEntityKind[] = ['service', 'datastore', 'queue', 'api', 'system', 'frontend', 'endpoint', 'external_provider', 'repository'];

function blank(kind: PortalEntityKind, name: string): PortalEntity {
  return {
    id: `${kind}:${name}`,
    kind,
    name,
    displayName: null,
    sources: [],
    inferred: false,
    owner: '',
    description: '',
    lifecycle: null,
    tier: null,
    entityType: null,
    languages: [],
    technology: null,
    tags: [],
    links: [],
    contacts: [],
    repoUrl: '',
    docsUrl: '',
    systems: [],
    dependsOn: [],
    service: null,
    related: [],
    requests: null,
    reqPerSec: null,
    errorRate: null,
    p95Ms: null,
    version: null,
    lastDeploy: null,
    slos: 0,
    monitors: 0,
    monitorsAlerting: 0,
    incidents: 0,
    openIncidents: 0,
    errorIssues: null,
    ciRuns: null,
    ciSuccessRate: null,
    lastCiRun: null,
    definitionId: null,
  };
}

const addSource = (e: PortalEntity, s: string) => {
  if (!e.sources.includes(s)) e.sources.push(s);
};

/** Every entity, merged from telemetry, service metadata and definitions, like the server. */
export function assembleCatalog(f: CatalogFacts): PortalEntity[] {
  const all = new Map<string, PortalEntity>();
  const get = (kind: PortalEntityKind, name: string) => {
    const id = `${kind}:${name}`;
    let e = all.get(id);
    if (!e) all.set(id, (e = blank(kind, name)));
    return e;
  };
  const instrumented = new Set(f.services.map((s) => s.name));
  for (const s of f.services) {
    const e = get('service', s.name);
    addSource(e, 'apm');
    Object.assign(e, {
      entityType: s.web ? 'web' : 'worker',
      languages: s.language ? [s.language] : [],
      version: s.version ?? null,
      lastDeploy: s.lastDeploy ?? null,
      requests: s.requests,
      reqPerSec: s.reqPerSec,
      errorRate: s.errorRate,
      p95Ms: s.p95Ms,
    });
  }
  for (const n of f.logServices) if (n) addSource(get('service', n), 'logs');
  for (const n of f.metricServices) if (n) addSource(get('service', n), 'metrics');
  for (const n of f.metadata.keys()) addSource(get('service', n), 'metadata');
  for (const p of f.peers) {
    const e = get(p.kind, p.name);
    addSource(e, 'spans');
    Object.assign(e, { technology: p.technology, entityType: p.technology, related: [...p.callers], requests: p.requests, errorRate: p.requests ? p.errors / p.requests : 0, p95Ms: p.p95Ms });
  }
  for (const { id, definition: d } of f.definitions) {
    const e = get(d.kind as PortalEntityKind, d.metadata.name);
    addSource(e, 'definition');
    const link = (t: string) => d.metadata.links?.find((l) => l.type === t)?.url ?? '';
    Object.assign(e, {
      definitionId: id,
      displayName: d.metadata.displayName ?? null,
      owner: d.metadata.owner ?? '',
      description: d.metadata.description ?? '',
      lifecycle: d.spec?.lifecycle ?? null,
      tier: d.spec?.tier ?? null,
      tags: [...(d.metadata.tags ?? [])],
      links: [...(d.metadata.links ?? [])],
      contacts: [...(d.metadata.contacts ?? [])],
      repoUrl: link('repo'),
      docsUrl: link('doc'),
      systems: (d.spec?.componentOf ?? []).filter((r) => r.startsWith('system:')).map((r) => r.slice(7)),
      dependsOn: [...(d.spec?.dependsOn ?? [])],
    });
    if (d.spec?.type) e.entityType = d.spec.type;
    if (d.spec?.languages?.length) e.languages = [...d.spec.languages];
    if (d.kind === 'system') e.related = [...(d.spec?.components ?? [])];
    if (d.kind === 'api' && d.spec?.implementedBy?.length) e.related = [...d.spec.implementedBy];
  }
  const systems = new Map<string, string[]>();
  for (const e of all.values()) for (const s of e.systems) systems.set(s, [...(systems.get(s) ?? []), `${e.kind}:${e.name}`]);
  for (const [name, members] of systems) {
    const e = get('system', name);
    if (!e.definitionId) addSource(e, 'definition');
    e.related = uniq([...e.related, ...members]).sort();
  }
  for (const e of all.values()) {
    if (e.kind !== 'service') continue;
    const meta = f.metadata.get(e.name);
    if (meta) {
      e.owner = meta.team;
      e.contacts = meta.contacts.map((c) => ({ ...(c.name ? { name: c.name } : {}), type: c.type, contact: c.value }));
      e.repoUrl = meta.repoUrl;
      e.docsUrl = meta.docsUrl;
    }
    const mentions = (t: string) => f.references(t, e.name);
    const monitors = f.monitors.filter((m) => m.texts.some(mentions));
    e.monitors = monitors.length;
    e.monitorsAlerting = monitors.filter((m) => m.alerting).length;
    e.slos = f.slos.filter((s) => s.texts.some(mentions) || (s.monitors ?? []).some((n) => monitors.some((m) => m.name === n))).length;
    const listed = f.incidents.filter((i) => i.services.includes(e.name));
    e.incidents = listed.filter((i) => i.inRange).length;
    e.openIncidents = listed.filter((i) => i.open).length;
    if (instrumented.has(e.name)) e.errorIssues = f.errorTypes[e.name] ?? 0;
  }
  for (const ep of f.endpoints) {
    const e = blank('endpoint', ep.resource);
    e.id = `endpoint:${ep.service}/${ep.resource}`;
    Object.assign(e, {
      sources: ['apm'],
      service: ep.service,
      owner: all.get(`service:${ep.service}`)?.owner ?? '',
      requests: ep.requests,
      reqPerSec: ep.reqPerSec,
      errorRate: ep.errorRate,
      p95Ms: ep.p95Ms,
    });
    // Its monitors: those naming its service and its route (`resource_name:"GET /orders"`, `/orders`).
    const route = (/^[A-Z]+\s+(\/.+)$/.exec(ep.resource)?.[1] ?? ep.resource).toLowerCase();
    const monitors = f.monitors.filter((m) => m.texts.some((t) => f.references(t, ep.service) && t.toLowerCase().includes(route)));
    e.monitors = monitors.length;
    e.monitorsAlerting = monitors.filter((m) => m.alerting).length;
    all.set(e.id, e);
  }
  const repos = new Map<string, PortalEntity>();
  const ci = new Map<string, { runs: number; failures: number; last: number }>();
  for (const r of f.repos) {
    const path = repoPath(r.url || r.name);
    if (!path) continue;
    let e = repos.get(path);
    if (!e) repos.set(path, (e = blank('repository', path)));
    addSource(e, 'ci');
    if (r.url) e.repoUrl = r.url;
    const c = ci.get(path) ?? { runs: 0, failures: 0, last: 0 };
    ci.set(path, { runs: c.runs + r.runs, failures: c.failures + r.failures, last: Math.max(c.last, r.lastRun) });
  }
  for (const [path, c] of ci) Object.assign(repos.get(path)!, { ciRuns: c.runs, ciSuccessRate: c.runs ? Math.max(0, c.runs - c.failures) / c.runs : null, lastCiRun: c.last });
  for (const e of all.values()) {
    if (!e.repoUrl || e.kind === 'endpoint') continue;
    const path = repoPath(e.repoUrl);
    if (!path) continue;
    let r = repos.get(path);
    if (!r) repos.set(path, (r = blank('repository', path)));
    addSource(r, e.definitionId && e.kind !== 'service' ? 'definition' : 'metadata');
    if (!r.repoUrl) r.repoUrl = e.repoUrl;
    const member = `${e.kind}:${e.name}`;
    if (!r.related.includes(member)) r.related.push(member);
    if (!r.owner) r.owner = e.owner;
  }
  for (const e of all.values()) {
    const c = e.kind === 'service' && e.repoUrl ? repos.get(repoPath(e.repoUrl)) : undefined;
    if (c) Object.assign(e, { ciRuns: c.ciRuns, ciSuccessRate: c.ciSuccessRate, lastCiRun: c.lastCiRun });
  }
  for (const [, r] of repos) all.set(r.id, r);
  const out = [...all.values()];
  for (const e of out) {
    e.inferred = !e.sources.some((s) => ['apm', 'definition', 'metadata', 'ci'].includes(s));
    e.sources.sort();
  }
  return out.sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) || a.id.localeCompare(b.id));
}

/** Entities owned per team handle (endpoints excluded). */
export function entitiesPerTeam(entities: PortalEntity[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const e of entities) if (e.owner && e.kind !== 'endpoint') out.set(e.owner, (out.get(e.owner) ?? 0) + 1);
  return out;
}

/** A service definition's owner, contacts, repository and docs as service metadata (they live there). */
export function metadataOf(def: EntityDefinition): ServiceMetaFact {
  const link = (t: string) => def.metadata.links?.find((l) => l.type === t)?.url ?? '';
  return {
    team: def.metadata.owner ?? '',
    contacts: (def.metadata.contacts ?? []).map((c) => ({ type: c.type === 'microsoft-teams' ? 'link' : c.type, name: c.name ?? '', value: c.contact })),
    repoUrl: link('repo'),
    docsUrl: link('doc'),
  };
}
