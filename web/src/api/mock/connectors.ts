// In-browser stand-in for Integrations → connectors (routes/connectors.rs). The catalog is every bundled package
// (web/src/api/connectors/packages.generated.json, `node scripts/connector.mjs build`) and the native connectors;
// connections live in memory for the session, a test passes with any secret of 8 characters or more and answers the
// package's recorded test message.
import type { components } from '../generated/schema';
import packages from '../connectors/packages.generated.json';
import { registerConnector } from '../integrations/collection';

type S = components['schemas'];
type Entry = S['ConnectorEntry'];
type View = S['ConnectionView'];
type Input = S['ConnectionInput'];

interface Package {
  entry: Entry;
  mock: { config: Record<string, unknown> | null; secrets: string[] | null; test: string | null };
  dashboards: unknown[];
}

const PACKAGES = packages as unknown as Package[];
// The bundled packages' tiles are available and link to their page, before the server's list arrives.
for (const p of PACKAGES) registerConnector(p.entry);

/** The native connectors (connectors/native.rs): their own pages. */
const NATIVE: Entry[] = [
  ['authentik', 'authentik', 'The instance’s events (logins, failed logins, policy and model changes) as logs.', '/integrations/authentik'],
  ['fleetdm', 'FleetDM', 'A Fleet server’s activities as logs; its hosts and policies as metrics.', '/integrations/fleetdm'],
  ['gitlab-audit-events', 'GitLab Audit Events', 'An instance’s, group’s or project’s audit events as logs.', '/integrations/gitlab-audit-events'],
  ['google-workspace', 'Google Workspace', 'The Reports API’s activities (logins, Drive, admin, tokens, …) as logs.', '/integrations/google-workspace'],
  ['azure-active-directory', 'Microsoft Entra ID', 'A tenant’s sign-in, audit and risk detection logs from Microsoft Graph.', '/integrations/azure?detail=entra'],
].map(([id, name, description, page]) => ({
  id,
  name,
  publisher: 'ObserveCraft',
  vendor: name,
  version: '1.0.0',
  description,
  tiles: [id],
  logo: id,
  categories: ['Security', 'Log Collection'],
  source: id,
  kind: 'native' as const,
  thirdParty: false,
  page,
  links: [],
  config: [],
  streams: [],
  dashboards: [],
  monitors: [],
  testable: true,
}));

const MIN = 60_000;
const connections = new Map<string, (View & { secretValues: Record<string, string> })[]>();
let seq = 0;

const delay = <T>(value: T, ms = 150) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));

function fail(message: string): never {
  throw new Error(message);
}

function entry(id: string): Entry {
  return PACKAGES.find((p) => p.entry.id === id)?.entry ?? NATIVE.find((n) => n.id === id) ?? fail(`No connector ${id}`);
}

function pkg(id: string): Package {
  const p = PACKAGES.find((x) => x.entry.id === id);
  if (!p) fail(NATIVE.some((n) => n.id === id) ? `${id} is set up on its own page` : `No connector ${id}`);
  return p;
}

/** The connector's bundled packages (catalog tiles a connector makes available). */
export const connectorEntries = (): Entry[] => [...NATIVE, ...PACKAGES.map((p) => p.entry)];

/** The bundled packages' own dashboards (dist/dashboards.json), for the mock's dashboards. */
export const connectorDashboards = (): unknown[] => PACKAGES.flatMap((p) => p.dashboards);

/** Whether the connector has a mock connection: its tiles are installed, as the server counts one. */
export const connectorConnected = (id: string): boolean => (connections.get(id) ?? []).length > 0;

function hostOf(url: unknown): string | null {
  try {
    return typeof url === 'string' ? new URL(url).hostname : null;
  } catch {
    return null;
  }
}

/** The input checked as the server checks it (types, https URLs, required fields, kept secrets). */
function checked(p: Package, input: Input, previous?: View & { secretValues: Record<string, string> }) {
  const e = p.entry;
  const config: Record<string, unknown> = {};
  let urlChanged = false;
  for (const f of e.config.filter((f) => !f.secret)) {
    let v = input.config?.[f.key] as unknown;
    if (typeof v === 'string') v = v.trim();
    if (f.type === 'url' && typeof v === 'string' && v) {
      v = v.replace(/\/+$/, '');
      if (!/^https:\/\/[^\s/@]+/.test(v as string)) fail(`${f.label}: The base URL must use https`);
    }
    if ((f.type === 'list' || f.type === 'multiselect') && typeof v === 'string')
      v = v
        .split(/[\n,]/)
        .map((s) => s.trim())
        .filter(Boolean);
    if (v === undefined || v === null || v === '' || (Array.isArray(v) && !v.length)) {
      if (f.required && (f.default === null || f.default === undefined)) fail(`${e.name} needs ${f.label}`);
      continue;
    }
    if (f.type === 'url' && previous && previous.config[f.key] !== v) urlChanged = true;
    config[f.key] = v;
  }
  const secretValues: Record<string, string> = {};
  for (const f of e.config.filter((f) => f.secret)) {
    const v = String(input.secrets?.[f.key] ?? '').trim();
    if (v) secretValues[f.key] = v;
    else if (previous?.secretValues[f.key]) {
      if (urlChanged) fail(`Enter ${f.label} again: the URL changed`);
      secretValues[f.key] = previous.secretValues[f.key];
    } else if (f.required) fail(`${e.name} needs ${f.label}`);
  }
  const streams = input.streams ?? previous?.streams.filter((s) => s.enabled).map((s) => s.id) ?? e.streams.filter((s) => s.defaultEnabled).map((s) => s.id);
  const firstUrl = e.config.find((f) => f.type === 'url');
  const name = input.name?.trim() || previous?.name || hostOf(config[firstUrl?.key ?? ''] ?? firstUrl?.default) || e.id;
  return { config, secretValues, streams, name };
}

function view(p: Package, id: string, name: string, config: Record<string, unknown>, secretValues: Record<string, string>, streams: string[], at: number) {
  return {
    id,
    name,
    config,
    secrets: Object.keys(secretValues),
    secretValues,
    enabled: true,
    updatedAt: at,
    streams: p.entry.streams.map((s) => ({
      id: s.id,
      name: s.name,
      enabled: streams.includes(s.id),
      lastSuccessAt: streams.includes(s.id) ? at : null,
      collectedUntil: streams.includes(s.id) ? at - 20_000 : null,
      records: streams.includes(s.id) ? 4 : 0,
      written: streams.includes(s.id) ? (s.metrics.length ? 21 : 4) : 0,
      error: null,
    })),
  };
}

const visible = (v: View & { secretValues?: Record<string, string> }): View => {
  const { secretValues: _hidden, ...rest } = v as View & { secretValues: Record<string, string> };
  void _hidden;
  return rest;
};

export function listConnectors(): Promise<S['ConnectorCatalog']> {
  return delay({ connectors: connectorEntries().sort((a, b) => a.name.localeCompare(b.name)), invalid: [] });
}

export function getConnector(id: string): Promise<S['ConnectorDetail']> {
  const now = Date.now();
  const list = (connections.get(id) ?? []).map((c) => ({ ...visible(c), streams: c.streams.map((s) => (s.enabled ? { ...s, lastSuccessAt: now - ((now - (c.updatedAt ?? now)) % MIN) } : s)) }));
  return delay({ connector: entry(id), connections: list });
}

export function createConnection(id: string, input: Input): Promise<View> {
  const p = pkg(id);
  const c = checked(p, input);
  const list = connections.get(id) ?? [];
  if (list.some((x) => x.name === c.name)) fail(`A connection named ${c.name} exists`);
  const v = view(p, `00000000-0000-4000-8000-${String(++seq).padStart(12, '0')}`, c.name, c.config, c.secretValues, c.streams, Date.now());
  connections.set(id, [...list, v]);
  return delay(visible(v), 250);
}

export function updateConnection(id: string, connection: string, input: Input): Promise<View> {
  const p = pkg(id);
  const list = connections.get(id) ?? [];
  const previous = list.find((x) => x.id === connection) ?? fail('No such connection');
  const c = checked(p, input, previous);
  const v = view(p, connection, c.name, c.config, c.secretValues, c.streams, Date.now());
  connections.set(
    id,
    list.map((x) => (x.id === connection ? v : x)),
  );
  return delay(visible(v), 250);
}

export function deleteConnection(id: string, connection: string): Promise<void> {
  const list = connections.get(id) ?? [];
  if (!list.some((x) => x.id === connection)) fail('No such connection');
  connections.set(
    id,
    list.filter((x) => x.id !== connection),
  );
  return delay(undefined);
}

export function syncConnection(id: string, connection: string): Promise<void> {
  const c = (connections.get(id) ?? []).find((x) => x.id === connection) ?? fail('No such connection');
  c.updatedAt = Date.now();
  return delay(undefined);
}

export function testConnection(id: string, input: Input): Promise<S['ConnectorTest']> {
  const p = pkg(id);
  const previous = input.connectionId ? (connections.get(id) ?? []).find((x) => x.id === input.connectionId) : undefined;
  const c = checked(p, input, previous);
  const short = Object.entries(c.secretValues).find(([, v]) => v.length < 8);
  if (short) return delay({ ok: false, message: `${p.entry.name} refused the credentials (HTTP 401)`, records: null }, 400);
  return delay({ ok: true, message: p.mock.test ? `Connected: ${p.mock.test}` : 'Connected', records: 4 }, 400);
}

export function createPush(id: string, input: Input): Promise<S['ConnectorTest']> {
  const p = pkg(id);
  if (!p.entry.push?.canCreate) fail(`${p.entry.name} sets up its push in its console`);
  checked(p, input);
  return delay({ ok: true, message: `Created on ${p.entry.vendor}: it sends to ${input.pushUrl ?? 'the intake'}`, records: null }, 400);
}

/** Mock mode has no vendor: the consent is granted at once and the connection saved, as the callback would. */
export async function startOAuth(id: string, input: Input): Promise<{ url: string }> {
  const v = input.connectionId ? await updateConnection(id, input.connectionId, input) : await createConnection(id, input);
  return delay({ url: `/integrations/connectors/${id}?oauth=${encodeURIComponent(v.name)}` });
}
