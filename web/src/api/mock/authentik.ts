// In-browser stand-in for Integrations → authentik (routes/authentik_integration.rs). The connection lives in
// memory for the session; any token of 20 characters or more is accepted. Once saved, the last sync follows the
// poller's minute.
import type { components } from '../generated/schema';

type S = components['schemas'];
type Input = S['AuthentikInput'];
type Status = S['AuthentikStatus'];
type Test = S['AuthentikTest'];

const MIN = 60_000;

let saved: { baseUrl: string; at: number } | null = null;
let syncedAt = 0;

const delay = <T>(value: T, ms = 150) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));

function fail(message: string): never {
  throw new Error(message);
}

function checkedUrl(input: Input): string {
  const baseUrl = (input.baseUrl ?? '').trim().replace(/\/+$/, '');
  if (!baseUrl) fail('authentik needs its URL (https://auth.example.com)');
  if (!/^https:\/\/[^\s/@]+/.test(baseUrl)) fail('The base URL must use https');
  return baseUrl;
}

function status(): Status {
  if (!saved) return { connection: null, lastSyncAt: null, syncedUntil: null, eventsLastSync: 0, error: null };
  const now = Date.now();
  const lastSync = Math.max(syncedAt, now - ((now - saved.at) % MIN));
  return { connection: { baseUrl: saved.baseUrl, updatedAt: saved.at }, lastSyncAt: lastSync, syncedUntil: lastSync - 20_000, eventsLastSync: 12, error: null };
}

export const authentikStatus = (): Promise<Status> => delay(status());

export function testAuthentik(input: Input): Promise<Test> {
  const baseUrl = checkedUrl(input);
  const token = input.token?.trim() || (saved?.baseUrl === baseUrl ? 'saved-token-of-the-mock-0000' : '');
  if (!token) fail('authentik needs an API token');
  if (token.length < 20) return delay({ ok: false, message: 'authentik refused the token: Token invalid/expired', user: null, events: null }, 400);
  return delay({ ok: true, message: 'Connected as observecraft-reader: 1842 events to read', user: 'observecraft-reader', events: 1842 }, 400);
}

export function saveAuthentik(input: Input): Promise<Status> {
  const baseUrl = checkedUrl(input);
  if (!input.token?.trim() && saved?.baseUrl !== baseUrl) fail(saved ? 'Enter the token again: the authentik URL changed' : 'authentik needs an API token');
  saved = { baseUrl, at: Date.now() };
  return delay(status(), 250);
}

export function deleteAuthentik(): Promise<void> {
  if (!saved) fail('authentik is not connected');
  saved = null;
  return delay(undefined);
}

export function syncAuthentik(): Promise<void> {
  if (!saved) fail('authentik is not connected');
  syncedAt = Date.now();
  return delay(undefined);
}
