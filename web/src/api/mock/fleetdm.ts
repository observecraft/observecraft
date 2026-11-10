// In-browser stand-in for Integrations → FleetDM (routes/fleetdm_integration.rs). The connection lives in memory
// for the session; any token of 20 characters or more is accepted. Once saved, the last syncs follow the poller's
// minute. `fleetdmLogs` is the mock activity feed (`source:fleetdm`), shaped as crate::fleetdm::logs makes it and the
// FleetDM log pipeline completes it (`fleetdm.category`).
import type { components } from '../generated/schema';
import type { JsonValue, LogEvent, LogStatus } from '../types';
import type { Rand } from './generator';

type S = components['schemas'];
type Input = S['FleetdmInput'];
type Status = S['FleetdmStatus'];
type Test = S['FleetdmTest'];

const MIN = 60_000;

let saved: { baseUrl: string; at: number } | null = null;
let syncedAt = 0;

const delay = <T>(value: T, ms = 150) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));

function fail(message: string): never {
  throw new Error(message);
}

function checkedUrl(input: Input): string {
  const baseUrl = (input.baseUrl ?? '').trim().replace(/\/+$/, '');
  if (!baseUrl) fail('Fleet needs its URL (https://fleet.example.com)');
  if (!/^https:\/\/[^\s/@]+/.test(baseUrl)) fail('The base URL must use https');
  return baseUrl;
}

function status(): Status {
  if (!saved) return { connection: null, lastSyncAt: null, syncedUntil: null, activitiesLastSync: 0, hostsSyncedAt: null, hosts: 0, error: null };
  const now = Date.now();
  const lastSync = Math.max(syncedAt, now - ((now - saved.at) % MIN));
  return { connection: { baseUrl: saved.baseUrl, updatedAt: saved.at }, lastSyncAt: lastSync, syncedUntil: lastSync - 20_000, activitiesLastSync: 9, hostsSyncedAt: lastSync, hosts: 248, error: null };
}

export const fleetdmStatus = (): Promise<Status> => delay(status());

/** Whether the mock connection is saved: the catalog tile is installed, as the server counts a saved connection. */
export const fleetdmConnected = (): boolean => !!saved;

export function testFleetdm(input: Input): Promise<Test> {
  const baseUrl = checkedUrl(input);
  const token = input.token?.trim() || (saved?.baseUrl === baseUrl ? 'saved-token-of-the-mock-0000' : '');
  if (!token) fail('Fleet needs an API token');
  if (token.length < 20) return delay({ ok: false, message: 'Fleet refused the token: Authentication required', user: null, role: null, hosts: null }, 400);
  return delay({ ok: true, message: 'Connected as ObserveCraft (observer): 248 hosts', user: 'ObserveCraft', role: 'observer', hosts: 248 }, 400);
}

export function saveFleetdm(input: Input): Promise<Status> {
  const baseUrl = checkedUrl(input);
  if (!input.token?.trim() && saved?.baseUrl !== baseUrl) fail(saved ? 'Enter the token again: the Fleet URL changed' : 'Fleet needs an API token');
  saved = { baseUrl, at: Date.now() };
  return delay(status(), 250);
}

export function deleteFleetdm(): Promise<void> {
  if (!saved) fail('Fleet is not connected');
  saved = null;
  return delay(undefined);
}

export function syncFleetdm(): Promise<void> {
  if (!saved) fail('Fleet is not connected');
  syncedAt = Date.now();
  return delay(undefined);
}

// ---------------------------------------------------------------- the activity feed

const HOUR = 60 * MIN;
const HOST = 'fleet.example.com';

/** id, name, email (fictional) */
const PEOPLE: [number, string, string][] = [
  [3, 'Jordan Patel', 'jordan.patel@example.com'],
  [5, 'Sam Lee', 'sam.lee@example.com'],
  [8, 'Alex Morgan', 'alex.morgan@example.com'],
];
const HOSTS: [number, string][] = [
  [1, 'Jordan’s MacBook Pro'],
  [3, 'Alex’s MacBook Air'],
  [5, 'desktop-sam'],
  [7, 'laptop-morgan'],
  [11, 'i-0a1b2c3d4e5f60004'],
  [12, 'build-runner-1'],
];
const pick = <T>(r: Rand, xs: T[]) => xs[Math.floor(r() * xs.length)];

/** type, weight, details */
const ACTIVITIES: [string, number, (r: Rand) => Record<string, JsonValue>][] = [
  ['user_logged_in', 30, () => ({ public_ip: '203.0.113.24' })],
  ['user_failed_login', 6, (r) => ({ email: pick(r, ['sam.lee@example.com', 'admin@example.com']), public_ip: '198.51.100.7' })],
  ['live_query', 18, (r) => ({ query_sql: 'SELECT * FROM usb_devices;', targets_count: 1 + Math.floor(r() * 12) })],
  ['ran_script', 14, (r) => ({ script_name: pick(r, ['rotate-logs.sh', 'clear-dns-cache.sh', 'collect-diagnostics.ps1']) })],
  ['installed_software', 14, (r) => ({ software_title: pick(r, ['zoom.us.app', 'Slack.app', '1Password.app']), status: r() < 0.15 ? 'failed_install' : 'installed', self_service: r() < 0.5 })],
  ['created_policy', 4, (r) => ({ policy_name: pick(r, ['Screen lock after 15 minutes', 'Firewall enabled']) })],
  ['edited_policy', 6, (r) => ({ policy_name: pick(r, ['Disk encryption enabled', 'osquery 5.13 or newer']) })],
  ['deleted_policy', 1, () => ({ policy_name: 'Legacy antivirus running' })],
  ['created_saved_query', 4, (r) => ({ query_name: pick(r, ['USB devices', 'Listening ports', 'Browser extensions']) })],
  ['edited_agent_options', 2, () => ({ global: true })],
  ['locked_host', 1, () => ({})],
  ['wiped_host', 1, () => ({})],
  ['read_host_disk_encryption_key', 2, () => ({})],
  ['transferred_hosts', 2, () => ({ team_name: 'Workstations' })],
  ['mdm_enrolled', 3, () => ({ host_serial: 'C02ABC123XYZ', installed_from_dep: true, mdm_platform: 'apple' })],
  ['created_macos_profile', 2, (r) => ({ profile_name: pick(r, ['Wi-Fi', 'FileVault', 'Screen lock']) })],
  ['enabled_macos_disk_encryption', 1, () => ({})],
];

/** The FleetDM log pipeline's category of an activity. */
function category(type: string): string {
  if (type.startsWith('user_')) return 'authentication';
  if (['ran_script', 'locked_host', 'wiped_host', 'read_host_disk_encryption_key', 'transferred_hosts'].includes(type)) return 'host_action';
  if (type.endsWith('_software')) return 'software';
  if (type.startsWith('mdm_') || type.endsWith('_profile') || type.endsWith('_disk_encryption')) return 'mdm';
  if (type.endsWith('_policy')) return 'policy';
  if (type === 'live_query' || type.endsWith('_saved_query')) return 'query';
  return 'configuration';
}
/** Activities about one host. */
const ON_HOST = ['ran_script', 'installed_software', 'locked_host', 'wiped_host', 'read_host_disk_encryption_key', 'mdm_enrolled'];
const TOTAL = ACTIVITIES.reduce((n, a) => n + a[1], 0);
const WARNINGS = ['locked_host', 'wiped_host', 'read_host_disk_encryption_key'];
const SUBJECTS = ['policy_name', 'query_name', 'script_name', 'software_title', 'team_name'];

/** A week of activities, as the FleetDM poller stores them. */
export function fleetdmLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  // A week, denser in the last hours so short ranges look alive.
  for (let i = 0; i < 340; i++) {
    const ts = Math.round(now - r() * (i >= 260 ? 3 * HOUR : 7 * 24 * HOUR));
    let w = r() * TOTAL;
    const [type, , make] = ACTIVITIES.find((a) => (w -= a[1]) < 0) ?? ACTIVITIES[0];
    const details = make(r);
    const target = ON_HOST.includes(type) ? pick(r, type === 'mdm_enrolled' ? HOSTS.slice(0, 2) : HOSTS) : null;
    if (target) Object.assign(details, { host_id: target[0], host_display_name: target[1] });
    const failedLogin = type === 'user_failed_login';
    const automatic = type === 'transferred_hosts' && r() < 0.5;
    const person = failedLogin || automatic ? null : pick(r, PEOPLE);
    const failed = failedLogin || String(details.status ?? '').includes('fail');
    const status: LogStatus = failed ? 'error' : WARNINGS.includes(type) || type.startsWith('deleted_') ? 'warn' : 'info';
    const verb = type.replace(/^user_(logged|added_by)/, '$1').replace(/_/g, ' ');
    const subject = SUBJECTS.map((k) => details[k]).find((v): v is string => typeof v === 'string');
    const where = target ? `${type.endsWith('_host') ? ' ' : ' on '}${target[1]}` : '';
    const message = failedLogin ? `Failed login for ${details.email}` : `${person?.[1] ?? 'Fleet'} ${verb}${subject ? ` ${subject}` : ''}${where}${details.status ? ` (${details.status})` : ''}`;
    const fleetdm: Record<string, JsonValue> = { activity_id: String(9000 + i), fleet_initiated: automatic, details, category: category(type) };
    if (target) Object.assign(fleetdm, { host_id: target[0], host_display_name: target[1] });
    if (typeof details.team_name === 'string') fleetdm.team = details.team_name;
    const attributes: Record<string, JsonValue> = { evt: { name: type, outcome: failed ? 'failure' : 'success' }, fleetdm };
    if (person) attributes.usr = { id: String(person[0]), name: person[1], email: person[2] };
    else if (failedLogin) attributes.usr = { email: String(details.email) };
    if (typeof details.public_ip === 'string') attributes.network = { client: { ip: details.public_ip } };
    out.push({ id: `fleetdm-${ts}-${i}`, timestamp: ts, status, service: 'fleetdm', host: HOST, source: 'fleetdm', message, tags: {}, attributes });
  }
  return out;
}
