// In-browser stand-in for Integrations → Google Workspace (routes/google_workspace_integration.rs). The connection
// lives in memory for the session; any JSON key of type service_account with a client_email is accepted, and one whose
// client_email starts with `no-delegation` is refused as Google refuses a key without domain-wide delegation. Once
// saved, each application's last sync follows the poller's five minutes.
// `googleWorkspaceLogs` is the mock activity log (`source:gsuite`, `service:` the application), shaped as
// crate::google_workspace::logs makes it and the Google Workspace log pipeline completes it (`google_workspace.*`).
import type { components } from '../generated/schema';
import type { JsonValue, LogEvent, LogStatus } from '../types';
import type { Rand } from './generator';

type S = components['schemas'];
type Input = S['GoogleWorkspaceInput'];
type Status = S['GoogleWorkspaceStatus'];
type Test = S['GoogleWorkspaceTest'];

const MIN = 60_000;
const INTERVAL = 5 * MIN;

/** The Reports API applications the integration reads (api.rs APPLICATIONS), the defaults marked. */
const APPLICATIONS: [string, string, boolean][] = [
  ['access_transparency', 'Access Transparency', false],
  ['admin', 'Admin console', true],
  ['calendar', 'Calendar', false],
  ['chat', 'Google Chat', false],
  ['chrome', 'Chrome', false],
  ['context_aware_access', 'Context-Aware Access', false],
  ['drive', 'Drive', true],
  ['gcp', 'Google Cloud', false],
  ['groups', 'Groups', true],
  ['groups_enterprise', 'Enterprise Groups', false],
  ['jamboard', 'Jamboard', false],
  ['keep', 'Keep', false],
  ['login', 'Login', true],
  ['meet', 'Meet', false],
  ['mobile', 'Mobile devices', true],
  ['rules', 'Rules', false],
  ['saml', 'SAML', true],
  ['takeout', 'Takeout', false],
  ['token', 'OAuth tokens', true],
  ['user_accounts', 'User accounts', true],
  ['vault', 'Vault', false],
];
const available = APPLICATIONS.map(([id, name, d]) => ({ id, name, default: d }));

let saved: { clientEmail: string; subject: string; applications: string[]; at: number } | null = null;
let syncedAt = 0;

const delay = <T>(value: T, ms = 150) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));

function fail(message: string): never {
  throw new Error(message);
}

function checked(input: Input): { subject: string; applications: string[] } {
  const subject = (input.subject ?? '').trim().toLowerCase();
  if (!/^[^\s@]+@[^\s@.]+(\.[^\s@.]+)+$/.test(subject)) fail('Enter the email of a Google Workspace administrator for the service account to act as');
  const picked = (input.applications ?? []).map((a) => a.trim()).filter(Boolean);
  const bad = picked.find((a) => !APPLICATIONS.some(([id]) => id === a));
  if (bad) fail(`${bad} is not a Reports API application this integration reads`);
  const applications = [...new Set(picked.length ? picked : APPLICATIONS.filter(([, , d]) => d).map(([id]) => id))].sort();
  return { subject, applications };
}

/** The key's service account, or the saved one when the key is left out. */
function clientEmail(input: Input): string {
  const raw = input.serviceAccountKey?.trim();
  if (!raw) return saved?.clientEmail ?? fail("Google Workspace needs the service account's JSON key");
  let key: Record<string, unknown>;
  try {
    key = JSON.parse(raw);
  } catch {
    fail("The key is not JSON: paste the service account's JSON key file");
  }
  if (key.type !== 'service_account') fail('The key is not a service account key (its type must be service_account)');
  if (typeof key.client_email !== 'string' || !key.client_email.includes('@')) fail('The key has no client_email');
  if (typeof key.private_key !== 'string' || !key.private_key) fail('The key has no private_key');
  return key.client_email;
}

function status(): Status {
  if (!saved) return { connection: null, syncs: [], available };
  const now = Date.now();
  const lastSync = Math.max(syncedAt, now - ((now - saved.at) % INTERVAL));
  return {
    connection: { clientEmail: saved.clientEmail, subject: saved.subject, applications: saved.applications, updatedAt: saved.at },
    syncs: saved.applications.map((application, i) => ({
      application,
      lastSyncAt: lastSync,
      syncedUntil: lastSync - 30_000,
      eventsLastSync: [12, 48, 7, 3, 21, 5, 2, 9][i % 8],
      catchingUp: false,
      error: null,
    })),
    available,
  };
}

export const googleWorkspaceStatus = (): Promise<Status> => delay(status());

/** Whether the mock connection is saved: the catalog tile is installed, as the server counts a saved connection. */
export const googleWorkspaceConnected = (): boolean => !!saved;

export function testGoogleWorkspace(input: Input): Promise<Test> {
  const c = checked(input);
  const email = clientEmail(input);
  if (email.startsWith('no-delegation'))
    return delay(
      {
        ok: false,
        message:
          'Google refused domain-wide delegation: in the Admin console (Security → Access and data control → API controls → Domain-wide delegation), authorize the service account’s client ID for https://www.googleapis.com/auth/admin.reports.audit.readonly (Client is unauthorized to retrieve access tokens using this method)',
        clientEmail: email,
      },
      400,
    );
  return delay({ ok: true, message: `Connected: ${email} reads the activities of ${c.applications.join(', ')} as ${c.subject}`, clientEmail: email }, 400);
}

export function saveGoogleWorkspace(input: Input): Promise<Status> {
  const c = checked(input);
  saved = { ...c, clientEmail: clientEmail(input), at: Date.now() };
  return delay(status(), 250);
}

export function deleteGoogleWorkspace(): Promise<void> {
  if (!saved) fail('Google Workspace is not connected');
  saved = null;
  return delay(undefined);
}

export function syncGoogleWorkspace(): Promise<void> {
  if (!saved) fail('Google Workspace is not connected');
  syncedAt = Date.now();
  return delay(undefined);
}

// ---------------------------------------------------------------- the activity log

const HOUR = 60 * MIN;

/** email, profile id (fictional) */
const PEOPLE: [string, string][] = [
  ['jordan.patel@example.com', '104857600000000000042'],
  ['sam.lee@example.com', '104857600000000000043'],
  ['alex.morgan@example.com', '104857600000000000044'],
  ['riley.chen@example.com', '104857600000000000045'],
  ['casey.nguyen@example.com', '104857600000000000046'],
  ['taylor.kim@example.com', '104857600000000000047'],
];
const ADMINS: [string, string][] = [['admin@example.com', '104857600000000000001'], PEOPLE[0]];
/** address, country, ASN */
const CLIENTS: [string, string, number][] = [
  ['203.0.113.24', 'US', 64500],
  ['203.0.113.80', 'US', 64500],
  ['198.51.100.14', 'RO', 64510],
  ['192.0.2.55', 'DE', 64520],
];
const ATTACKERS: [string, string, number][] = [
  ['198.51.100.7', 'NL', 64501],
  ['192.0.2.200', 'BR', 64530],
  ['203.0.113.150', 'VN', 64540],
];
const DOCS = ['Q3 storefront roadmap', 'Checkout latency review', 'Order service runbook', 'Billing migration plan', 'Catalog pricing sheet', 'Inventory forecast 2027'];
const OAUTH_APPS = ['Example CRM', 'Calendar Sync Pro', 'Slide Builder', 'Expense Tracker'];
const GROUPS = ['engineering@example.com', 'storefront-oncall@example.com', 'finance@example.com'];
const pick = <T>(r: Rand, xs: T[]) => xs[Math.floor(r() * xs.length)];

/** application, event type, event name, weight */
const EVENTS: [string, string, string, number][] = [
  ['login', 'login', 'login_success', 30],
  ['login', 'login', 'login_failure', 10],
  ['login', 'login', 'login_challenge', 4],
  ['login', 'login', 'logout', 6],
  ['login', 'login', 'suspicious_login', 1],
  ['login', 'login', 'login_verification', 2],
  ['user_accounts', '2sv_change', '2sv_enroll', 2],
  ['user_accounts', '2sv_change', '2sv_disable', 1],
  ['user_accounts', 'password_change', 'password_edit', 2],
  ['user_accounts', 'recovery_info_change', 'recovery_email_edit', 1],
  ['admin', 'DELEGATED_ADMIN_SETTINGS', 'ASSIGN_ROLE', 1],
  ['admin', 'USER_SETTINGS', 'CREATE_USER', 2],
  ['admin', 'USER_SETTINGS', 'SUSPEND_USER', 1],
  ['admin', 'USER_SETTINGS', 'CHANGE_PASSWORD', 1],
  ['admin', 'APPLICATION_SETTINGS', 'CHANGE_APPLICATION_SETTING', 2],
  ['admin', 'SECURITY_SETTINGS', 'ENFORCE_STRONG_AUTHENTICATION', 1],
  ['admin', 'GROUP_SETTINGS', 'ADD_GROUP_MEMBER', 2],
  ['drive', 'access', 'view', 20],
  ['drive', 'access', 'edit', 10],
  ['drive', 'access', 'download', 5],
  ['drive', 'access', 'create', 4],
  ['drive', 'acl_change', 'change_document_visibility', 4],
  ['drive', 'acl_change', 'change_user_access', 3],
  ['token', 'auth', 'authorize', 6],
  ['token', 'auth', 'revoke', 2],
  ['groups_enterprise', 'moderator_action', 'add_member', 3],
  ['groups_enterprise', 'moderator_action', 'remove_member', 1],
  ['groups', 'user_action', 'join', 2],
  ['mobile', 'device_registration', 'DEVICE_REGISTER_UNREGISTER_EVENT', 2],
  ['mobile', 'device_sync', 'DEVICE_SYNC_EVENT', 4],
  ['mobile', 'device_updates', 'DEVICE_COMPROMISED_EVENT', 1],
  ['saml', 'login', 'login_success', 4],
  ['saml', 'login', 'login_failure', 1],
];
const TOTAL = EVENTS.reduce((n, e) => n + e[3], 0);

/** The pipeline's category of an application (google_workspace.category). */
function category(application: string): string {
  if (['login', 'saml', 'user_accounts'].includes(application)) return 'authentication';
  if (application === 'token') return 'authorization';
  if (application === 'admin') return 'administration';
  if (['groups', 'groups_enterprise'].includes(application)) return 'groups';
  if (['drive', 'takeout', 'vault', 'keep', 'calendar', 'chat', 'meet', 'jamboard', 'gplus'].includes(application)) return 'data_access';
  if (['mobile', 'chrome', 'context_aware_access', 'access_evaluation'].includes(application)) return 'devices';
  return 'other';
}

/** The poller's status of an event (logs.rs is_error / is_warning). */
function statusOf(name: string): LogStatus {
  const n = name.toLowerCase();
  if (['suspicious', 'gov_attack', 'hijack', 'leak', 'account_disabled', 'compromised'].some((w) => n.includes(w))) return 'error';
  if (['failure', 'failed', 'blocked', 'denied'].some((w) => n.includes(w)) || n === '2sv_disable') return 'warn';
  return 'info';
}

const words = (name: string) => {
  const s = name.toLowerCase().replaceAll('_', ' ');
  return s.charAt(0).toUpperCase() + s.slice(1);
};
const ABOUT = ['doc_title', 'USER_EMAIL', 'GROUP_EMAIL', 'ROLE_NAME', 'app_name', 'application_name', 'SETTING_NAME', 'DEVICE_MODEL', 'visibility', 'login_type'];

/** A week of Google Workspace activities, as the Google Workspace poller stores them. */
export function googleWorkspaceLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (let i = 0; i < 900; i++) {
    // A week, denser in the last hours so short ranges look alive.
    const ts = Math.round(now - r() * (i < 300 ? 3 * HOUR : 7 * 24 * HOUR));
    let w = r() * TOTAL;
    const [application, type, name] = EVENTS.find((e) => (w -= e[3]) < 0) ?? EVENTS[0];
    let [email, profile] = application === 'admin' ? pick(r, ADMINS) : pick(r, PEOPLE);
    let [ip, country, asn] = pick(r, CLIENTS);
    const p: Record<string, JsonValue> = {};
    if (application === 'login' || application === 'saml') {
      if (application === 'saml') p.application_name = pick(r, ['Example HR', 'Expense Tracker', 'ObserveCraft']);
      else p.login_type = r() < 0.85 ? 'google_password' : 'saml';
      if (name === 'login_failure') {
        [ip, country, asn] = r() < 0.7 ? pick(r, ATTACKERS) : [ip, country, asn];
        if (r() < 0.5) [email, profile] = pick(r, PEOPLE.slice(1, 4));
        p.login_failure_type = 'login_failure_invalid_password';
      }
      if (name === 'suspicious_login') {
        [ip, country, asn] = pick(r, ATTACKERS);
        p.is_suspicious = true;
      }
      if (name === 'login_success' && application === 'login') p.login_challenge_method = r() < 0.7 ? ['password', 'google_authenticator'] : ['password'];
      if (name === 'login_challenge') p.login_challenge_status = r() < 0.8 ? 'Challenge Passed' : 'Challenge Failed';
    } else if (application === 'admin') {
      const target = pick(r, PEOPLE);
      if (name === 'ASSIGN_ROLE') {
        p.ROLE_NAME = pick(r, ['_SEED_ADMIN_ROLE', 'Help Desk Admin', 'Groups Admin']);
        p.USER_EMAIL = target[0];
      } else if (name === 'ADD_GROUP_MEMBER') {
        p.GROUP_EMAIL = pick(r, GROUPS);
        p.USER_EMAIL = target[0];
      } else if (name === 'CHANGE_APPLICATION_SETTING') {
        p.APPLICATION_NAME = pick(r, ['Drive and Docs', 'Gmail', 'Calendar']);
        p.SETTING_NAME = pick(r, ['Sharing outside of example.com', 'Allow users to publish files on the web', 'External calendar sharing']);
        p.OLD_VALUE = 'false';
        p.NEW_VALUE = 'true';
      } else if (name === 'ENFORCE_STRONG_AUTHENTICATION') {
        p.ORG_UNIT_NAME = '/Engineering';
        p.NEW_VALUE = 'true';
      } else p.USER_EMAIL = target[0];
    } else if (application === 'drive') {
      const doc = pick(r, DOCS);
      p.doc_id = `1${doc.replaceAll(' ', '').slice(0, 12)}Xy${Math.floor(r() * 90 + 10)}`;
      p.doc_title = doc;
      p.doc_type = pick(r, ['document', 'spreadsheet', 'presentation']);
      p.owner = email;
      if (name === 'change_document_visibility') {
        const visibility = pick(r, ['people_with_link', 'people_with_link', 'public_on_the_web', 'people_within_domain_with_link']);
        p.visibility = visibility;
        p.old_value = ['private'];
        p.new_value = [visibility];
        p.visibility_change = visibility === 'people_within_domain_with_link' ? 'internal' : 'external';
      } else if (name === 'change_user_access') {
        p.visibility = 'shared_externally';
        p.target_user = pick(r, ['partner@example.net', 'contractor@example.org']);
        p.old_value = ['none'];
        p.new_value = ['can_view'];
        p.visibility_change = 'external';
      } else p.visibility = pick(r, ['private', 'people_within_domain_with_link', 'shared_internally']);
    } else if (application === 'token') {
      p.app_name = pick(r, OAUTH_APPS);
      p.client_id = `${100000 + OAUTH_APPS.indexOf(p.app_name as string)}.apps.googleusercontent.com`;
      p.scope = ['https://www.googleapis.com/auth/drive.readonly', 'https://www.googleapis.com/auth/calendar'];
    } else if (application === 'groups' || application === 'groups_enterprise') {
      p.group_email = pick(r, GROUPS);
      if (name !== 'join') p.member_id = pick(r, PEOPLE)[0];
    } else if (application === 'mobile') {
      p.DEVICE_MODEL = pick(r, ['Pixel 9', 'iPhone 16', 'Galaxy S25']);
      p.OS_VERSION = pick(r, ['Android 16', 'iOS 19.1']);
      p.DEVICE_TYPE = String(p.DEVICE_MODEL).startsWith('iPhone') ? 'IOS_SYNC' : 'ANDROID';
    } else if (name === '2sv_disable' || name === '2sv_enroll') {
      p.method = 'google_authenticator';
    }
    const status = statusOf(name);
    const evt: Record<string, JsonValue> = { name };
    if (status === 'warn' && name !== '2sv_disable') evt.outcome = 'failure';
    else if (name.endsWith('_success')) evt.outcome = 'success';
    const about = ABOUT.filter((k) => typeof p[k] === 'string' && p[k]).map((k) => `${k}: ${p[k]}`);
    const time = new Date(ts).toISOString();
    const unique = `-${(9_000_000_000_000_000 + i * 7_919).toString()}`;
    out.push({
      id: `google-workspace-${ts}-${i}`,
      timestamp: ts,
      status,
      service: application,
      host: '',
      source: 'gsuite',
      message: `${words(name)} (${application}) by ${email}${about.length ? ` [${about.join(', ')}]` : ''}`,
      tags: {},
      attributes: {
        evt,
        usr: { email, id: profile },
        network: { client: { ip, geoip: { country: { iso_code: country } }, asn: { number: asn } } },
        id: { time, uniqueQualifier: unique, applicationName: application, customerId: 'C01example' },
        actor: { email, profileId: profile, callerType: 'USER' },
        event: { type, name, parameters: p },
        google_workspace: { category: category(application), severity: status },
      },
    });
  }
  return out;
}
