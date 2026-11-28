// In-browser stand-in for Integrations → Vanta (routes/vanta_integration.rs). The connection lives in memory for the
// session; any client secret of 20 characters or more is accepted. Once saved, each configured type shows the
// members of the mock org as sent and accepted.
import type { components } from '../generated/schema';

type S = components['schemas'];
type Input = S['VantaInput'];
type Status = S['VantaStatus'];
type Test = S['VantaTest'];

/** The resource types the server sends (crate::vanta::sync::TYPES), their base type and what the mock org sends of each. */
const TYPES: [string, string, number][] = [
  ['user_account', 'UserAccount', 14],
  ['vulnerable_component', 'VulnerableComponent', 23],
  ['package_vulnerability', 'PackageVulnerability', 187],
  ['security_task', 'SecurityTask', 41],
  ['monitor', 'CustomResource', 36],
  ['detection_rule', 'CustomResource', 58],
  ['log_index', 'CustomResource', 3],
];

/** The custom resources' schemas (crate::vanta::custom::schema). */
const SCHEMAS: Record<string, [string, string][]> = {
  monitor: [
    ['monitorType', 'string'],
    ['muted', 'boolean'],
    ['notifies', 'boolean'],
    ['priority', 'string'],
    ['query', 'string'],
    ['recipients', 'string'],
    ['status', 'string'],
    ['tags', 'string'],
  ],
  detection_rule: [
    ['contentPack', 'string'],
    ['enabled', 'boolean'],
    ['isDefault', 'boolean'],
    ['notifies', 'boolean'],
    ['query', 'string'],
    ['severity', 'string'],
    ['tags', 'string'],
  ],
  log_index: [
    ['dailyLimitMillions', 'int32'],
    ['exclusionFilters', 'int32'],
    ['filter', 'string'],
    ['flexRetentionDays', 'int32'],
    ['position', 'int32'],
    ['retentionDays', 'int32'],
  ],
};

const schema = (kind: string): string | null => (SCHEMAS[kind] ? JSON.stringify({ properties: Object.fromEntries(SCHEMAS[kind].map(([name, type]) => [name, { type }])) }, null, 2) : null);

let saved: { region: string; clientId: string; resourceIds: Record<string, string>; at: number } | null = null;
let syncedAt = 0;

const delay = <T>(value: T, ms = 150) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));

function fail(message: string): never {
  throw new Error(message);
}

function checked(input: Input): { region: string; clientId: string; resourceIds: Record<string, string> } {
  const region = (input.region ?? '').trim();
  if (region !== 'us' && region !== 'gov') fail('The region must be us (api.vanta.com) or gov (api.vanta-gov.com)');
  const clientId = (input.clientId ?? '').trim();
  if (!clientId) fail('Vanta needs the app’s client id');
  const resourceIds: Record<string, string> = {};
  for (const [t, id] of Object.entries(input.resourceIds ?? {})) {
    if (!id?.trim()) continue;
    if (!TYPES.some(([k]) => k === t)) fail(`ObserveCraft doesn't send ${t} to Vanta (it sends ${TYPES.map(([k]) => k).join(', ')})`);
    resourceIds[t] = id.trim();
  }
  if (resourceIds.package_vulnerability && !resourceIds.vulnerable_component) fail('Vulnerabilities name their vulnerable components: send the vulnerable components too');
  return { region, clientId, resourceIds };
}

function status(): Status {
  if (!saved) {
    return {
      connection: null,
      lastSyncAt: null,
      error: null,
      types: TYPES.map(([t, base]) => ({ resourceType: t, baseType: base, schema: schema(t), configured: false, syncedAt: null, sent: 0, accepted: 0, rejected: 0, error: null })),
    };
  }
  // The first sync a few seconds after saving, or when Sync now was pressed since.
  const at = syncedAt > saved.at ? syncedAt : saved.at + 20_000;
  const synced = Date.now() >= at;
  return {
    connection: { region: saved.region, clientId: saved.clientId, resourceIds: saved.resourceIds, updatedAt: saved.at },
    lastSyncAt: synced ? at : null,
    error: null,
    types: TYPES.map(([t, base, count]) => {
      const configured = !!saved?.resourceIds[t];
      const common = { resourceType: t, baseType: base, schema: schema(t), configured };
      return configured && synced ? { ...common, syncedAt: at, sent: count, accepted: count, rejected: 0, error: null } : { ...common, syncedAt: null, sent: 0, accepted: 0, rejected: 0, error: null };
    }),
  };
}

export const vantaStatus = (): Promise<Status> => delay(status());

/** Whether the mock connection is saved: the catalog tile is installed, as the server counts a saved connection. */
export const vantaConnected = (): boolean => !!saved;

export function testVanta(input: Input): Promise<Test> {
  const c = checked(input);
  const secret = input.clientSecret?.trim() || (saved?.clientId === c.clientId ? 'saved-secret-of-the-mock-0000' : '');
  if (!secret) fail('Vanta needs the app’s client secret');
  if (secret.length < 20) return delay({ ok: false, message: 'Vanta refused the credentials: Vanta answered 401 Unauthorized: invalid_client' }, 400);
  return delay({ ok: true, message: `Vanta accepted the app’s credentials (${c.region === 'gov' ? 'https://api.vanta-gov.com' : 'https://api.vanta.com'})` }, 400);
}

export function saveVanta(input: Input): Promise<Status> {
  const c = checked(input);
  if (!input.clientSecret?.trim() && saved?.clientId !== c.clientId) fail(saved ? 'Enter the client secret again: the client id changed' : 'Vanta needs the app’s client secret');
  saved = { ...c, at: Date.now() };
  return delay(status(), 250);
}

export function deleteVanta(): Promise<void> {
  if (!saved) fail('Vanta is not connected');
  saved = null;
  return delay(undefined);
}

export function syncVanta(): Promise<void> {
  if (!saved) fail('Vanta is not connected');
  syncedAt = Date.now();
  return delay(undefined);
}
