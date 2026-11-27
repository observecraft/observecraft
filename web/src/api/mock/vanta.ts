// In-browser stand-in for Integrations → Vanta (routes/vanta_integration.rs). The connection lives in memory for the
// session; any client secret of 20 characters or more is accepted. Once saved, each configured type shows the
// members of the mock org as sent and accepted.
import type { components } from '../generated/schema';

type S = components['schemas'];
type Input = S['VantaInput'];
type Status = S['VantaStatus'];
type Test = S['VantaTest'];

/** The resource types the server sends (crate::vanta::sync::TYPES). */
const TYPES = ['user_account'];
const MEMBERS = 14;

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
    if (!TYPES.includes(t)) fail(`ObserveCraft doesn't send ${t} to Vanta (it sends ${TYPES.join(', ')})`);
    resourceIds[t] = id.trim();
  }
  return { region, clientId, resourceIds };
}

function status(): Status {
  if (!saved) {
    return { connection: null, lastSyncAt: null, error: null, types: TYPES.map((t) => ({ resourceType: t, configured: false, syncedAt: null, sent: 0, accepted: 0, rejected: 0, error: null })) };
  }
  // The first sync a few seconds after saving, or when Sync now was pressed since.
  const at = syncedAt > saved.at ? syncedAt : saved.at + 20_000;
  const synced = Date.now() >= at;
  return {
    connection: { region: saved.region, clientId: saved.clientId, resourceIds: saved.resourceIds, updatedAt: saved.at },
    lastSyncAt: synced ? at : null,
    error: null,
    types: TYPES.map((t) => {
      const configured = !!saved?.resourceIds[t];
      return configured && synced
        ? { resourceType: t, configured, syncedAt: at, sent: MEMBERS, accepted: MEMBERS, rejected: 0, error: null }
        : { resourceType: t, configured, syncedAt: null, sent: 0, accepted: 0, rejected: 0, error: null };
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
