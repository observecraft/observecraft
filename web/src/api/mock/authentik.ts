// In-browser stand-in for Integrations → authentik (routes/authentik_integration.rs). The connection lives in
// memory for the session; any token of 20 characters or more is accepted. Once saved, the last sync follows the
// poller's minute. `authentikLogs` is the mock event log (`source:authentik`), shaped as crate::authentik::logs makes
// it and the authentik log pipeline completes it (`authentik.category`, the attempted user of failed logins).
import type { components } from '../generated/schema';
import type { JsonValue, LogEvent, LogStatus } from '../types';
import type { Rand } from './generator';

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

/** Whether the mock connection is saved: the catalog tile is installed, as the server counts a saved connection. */
export const authentikConnected = (): boolean => !!saved;

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

// ---------------------------------------------------------------- the event log

const HOUR = 60 * MIN;
const HOST = 'auth.example.com';

/** pk, username, email (fictional) */
const PEOPLE: [number, string, string][] = [
  [42, 'jordan.patel', 'jordan.patel@example.com'],
  [43, 'sam.lee', 'sam.lee@example.com'],
  [44, 'alex.morgan', 'alex.morgan@example.com'],
  [45, 'riley.chen', 'riley.chen@example.com'],
  [46, 'casey.nguyen', 'casey.nguyen@example.com'],
];
const ADMINS = PEOPLE.slice(0, 2);
/** ip, country, city, continent, lat, long, ASN organization (documentation ranges) */
const CLIENTS: [string, string, string, string, number, number, string][] = [
  ['203.0.113.24', 'RO', 'Bucharest', 'EU', 44.43, 26.1, 'Example ISP'],
  ['203.0.113.80', 'DE', 'Berlin', 'EU', 52.52, 13.4, 'Example Telecom'],
  ['198.51.100.14', 'US', 'Seattle', 'NA', 47.61, -122.33, 'Example Cloud'],
  ['192.0.2.55', 'GB', 'London', 'EU', 51.51, -0.13, 'Example Broadband'],
];
/** Where failed logins and suspicious requests come from. */
const ATTACKERS: [string, string, string, string, number, number, string][] = [
  ['198.51.100.7', 'NL', 'Amsterdam', 'EU', 52.37, 4.9, 'Example Hosting'],
  ['192.0.2.200', 'BR', 'São Paulo', 'SA', -23.55, -46.63, 'Example Transit'],
];
const APPS = ['ObserveCraft', 'Grafana dev', 'Argo CD', 'GitLab', 'Vault'];
const DEVICES = ['totpdevice', 'webauthndevice', 'staticdevice'];
const MODELS: [string, string, string][] = [
  ['authentik_core', 'user', 'casey.nguyen'],
  ['authentik_core', 'group', 'platform-admins'],
  ['authentik_core', 'application', 'Argo CD'],
  ['authentik_providers_oauth2', 'oauth2provider', 'Provider for GitLab'],
  ['authentik_flows', 'flow', 'default-authentication-flow'],
  ['authentik_policies_expression', 'expressionpolicy', 'require-vpn'],
];
const pick = <T>(r: Rand, xs: T[]) => xs[Math.floor(r() * xs.length)];

/** action, weight */
const ACTIONS: [string, number][] = [
  ['login', 40],
  ['login_failed', 12],
  ['logout', 10],
  ['authorize_application', 22],
  ['model_updated', 5],
  ['model_created', 4],
  ['model_deleted', 1],
  ['password_set', 2],
  ['suspicious_request', 2],
  ['impersonation_started', 1],
  ['access_request_created', 1],
  ['access_request_denied', 1],
  ['policy_exception', 1],
  ['system_task_exception', 1],
  ['configuration_warning', 1],
];
const TOTAL = ACTIONS.reduce((n, a) => n + a[1], 0);
const ERRORS = ['login_failed', 'suspicious_request', 'system_exception', 'system_task_exception', 'policy_exception', 'property_mapping_exception', 'configuration_error'];
const WARNINGS = ['configuration_warning', 'access_request_denied', 'impersonation_started'];

/** The authentik log pipeline's category of an event. */
function category(action: string, modelName?: string): string {
  if (action.startsWith('model_')) return modelName && DEVICES.includes(modelName) ? 'mfa' : 'configuration_change';
  if (['login', 'login_failed', 'logout'].includes(action)) return 'authentication';
  if (action === 'password_set') return 'password';
  if (action.startsWith('impersonation_')) return 'impersonation';
  if (action === 'authorize_application') return 'application_access';
  if (action.startsWith('access_request_')) return 'access_request';
  if (action === 'suspicious_request') return 'suspicious';
  if (action.startsWith('policy_')) return 'policy';
  return 'system';
}

/** A week of events, as the authentik poller stores them. */
export function authentikLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  // A week, denser in the last hours so short ranges look alive.
  for (let i = 0; i < 700; i++) {
    const ts = Math.round(now - r() * (i < 250 ? 3 * HOUR : 7 * 24 * HOUR));
    let w = r() * TOTAL;
    const [action] = ACTIONS.find((a) => (w -= a[1]) < 0) ?? ACTIONS[0];
    const hostile = action === 'login_failed' ? r() < 0.6 : action === 'suspicious_request';
    const [ip, country, city, continent, lat, long, asn] = hostile ? pick(r, ATTACKERS) : pick(r, CLIENTS);
    const admin = action.startsWith('model_') || action.startsWith('impersonation_');
    let person = admin ? pick(r, ADMINS) : pick(r, PEOPLE);
    const context: Record<string, JsonValue> = {};
    let message = '';
    if (action === 'login') {
      context.auth_method = pick(r, ['password', 'password', 'password', 'auth_webauthn_pwl', 'token']);
      message = `${person[1]} logged in (${context.auth_method})`;
    } else if (action === 'login_failed') {
      const tried = hostile ? pick(r, ['admin', 'root', 'akadmin', person[1]]) : person[1];
      context.username = tried;
      context.stage = { name: pick(r, ['default-authentication-password', 'default-authentication-password', 'default-authentication-mfa-validation']) };
      message = `Login failed for ${tried}`;
      person = [0, tried, ''];
    } else if (action === 'logout') message = `${person[1]} logged out`;
    else if (action === 'authorize_application') {
      const app = pick(r, APPS);
      context.authorized_application = { name: app };
      message = `${person[1]} opened ${app}`;
    } else if (action.startsWith('model_')) {
      const enrolled = action !== 'model_updated' && r() < 0.4;
      const [app, model, name] = enrolled ? ['authentik_stages_authenticator', pick(r, DEVICES), 'security key'] : pick(r, MODELS);
      context.model = { app, model_name: model, name };
      message = `${person[1]} ${action.slice(6)} ${model} ${name}`;
    } else if (action === 'password_set') message = `Password set (${person[1]})`;
    else if (action === 'suspicious_request') {
      context.message = 'Too many failed attempts from this address';
      message = `Suspicious request: ${context.message}`;
    } else if (action === 'impersonation_started') message = `Impersonation started (${person[1]})`;
    else if (action.startsWith('access_request_')) {
      context.target = { name: pick(r, APPS) };
      message = `Access request ${action.slice(15)} for ${(context.target as { name: string }).name} (${person[1]})`;
    } else if (action === 'policy_exception') {
      context.message = 'expression policy require-vpn: NameError';
      message = `Policy exception: ${context.message}`;
    } else if (action === 'system_task_exception') {
      context.message = 'outpost authentik-embedded-outpost unreachable';
      message = `System task exception: ${context.message}`;
    } else {
      context.message = 'certificate for auth.example.com expires in 14 days';
      message = `Configuration warning: ${context.message}`;
    }
    const failure = ERRORS.includes(action) || action === 'access_request_denied';
    const status: LogStatus = ERRORS.includes(action) ? 'error' : WARNINGS.includes(action) ? 'warn' : 'info';
    const modelName = (context.model as { model_name?: string } | undefined)?.model_name;
    const authentik: Record<string, JsonValue> = { event_id: `ak-${ts}-${i}`, app: 'authentik.events.signals', brand: 'Example portal', context, category: category(action, modelName) };
    if (typeof context.authorized_application === 'object') authentik.application = (context.authorized_application as { name: string }).name;
    const usr: Record<string, JsonValue> = { name: person[1] };
    if (person[0]) Object.assign(usr, { id: String(person[0]), email: person[2] });
    const attributes: Record<string, JsonValue> = {
      evt: { name: action, outcome: failure ? 'failure' : 'success' },
      usr,
      network: {
        client: { ip, geoip: { country: { iso_code: country }, city: { name: city }, continent: { code: continent }, location: { latitude: lat, longitude: long } }, asn: { organization: asn } },
      },
      http: { method: 'POST', url_details: { path: '/api/v3/flows/executor/default-authentication-flow/' }, useragent: 'Mozilla/5.0' },
      authentik,
    };
    out.push({ id: `authentik-${ts}-${i}`, timestamp: ts, status, service: 'authentik', host: HOST, source: 'authentik', message, tags: {}, attributes });
  }
  return out;
}
