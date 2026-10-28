// In-browser identity and organization settings for mock mode: the current user's profile, users,
// roles, teams, SAML group mappings, sign-in settings, sessions, application keys, ingest API keys
// and the audit trail. State lives in memory for the session, so changes show up in later reads.
// Semantics follow the server (routes/identity.rs, auth.rs, sso.rs, ingest_keys.rs): org-scoped,
// secrets returned only when a key is created, invitations create pending users, changes audited.
// The people are the fictional ones the other mocks use (Alex Morgan, Priya Shah, …).
import type { components } from '../generated/schema';

type S = components['schemas'];

const DAY = 86_400_000;
export const MOCK_ORG = { id: 'default', name: 'Example Shop' };
export const ME_ID = '00000000-0000-0000-0000-000000000001';
const CLAIM = 'http://schemas.microsoft.com/ws/2008/06/identity/claims/groups';

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

const r = rng(20261025);
const pick = <T>(items: readonly T[]): T => items[Math.floor(r() * items.length)];
const hex = (n: number) => Array.from({ length: n }, () => Math.floor(r() * 16).toString(16)).join('');
const uuid = () => `${hex(8)}-${hex(4)}-4${hex(3)}-a${hex(3)}-${hex(12)}`;
const URL_SAFE = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
/** Like the server's `random_token`: a prefix and 43 URL-safe characters. */
const token = (prefix: string) => prefix + Array.from({ length: 43 }, () => URL_SAFE[Math.floor(r() * 64)]).join('');
const origin = () => (typeof location !== 'undefined' ? location.origin : 'http://localhost:5173');
const fail = (message: string): never => {
  throw new Error(message);
};
const ilike = (hay: string, needle: string) => hay.toLowerCase().includes(needle.toLowerCase());
const byName = (a: { name: string }, b: { name: string }) => a.name.toLowerCase().localeCompare(b.name.toLowerCase());
const clamp = (limit: number) => Math.min(200, Math.max(1, limit));

// ================================================================ seed

interface UserRow {
  id: string;
  name: string;
  email: string;
  title: string | null;
  status: S['UserStatus'];
  loginMethods: string[];
  mfa: boolean;
  timezone: string | null;
  lastLoginAt: number | null;
  createdAt: number;
  roleIds: string[];
}

interface RoleRow {
  id: string;
  name: string;
  description: string;
  managed: boolean;
}

interface TeamRow {
  handle: string;
  name: string;
  description: string;
  restricted: boolean;
  createdVia: string;
  notifications: string[];
  parent: string | null;
  /** In join order; admins listed first in search results. */
  members: { userId: string; role: 'admin' | 'member' }[];
}

const org = MOCK_ORG.id;
const ROLES: RoleRow[] = [
  { id: `${org}-admin`, name: 'ObserveCraft Admin Role', description: 'Full access, including organization settings, users and billing.', managed: true },
  { id: `${org}-standard`, name: 'ObserveCraft Standard Role', description: 'Create and edit dashboards, monitors and incidents; read all telemetry.', managed: true },
  { id: `${org}-readonly`, name: 'ObserveCraft Read Only Role', description: 'Read-only access to telemetry and assets.', managed: true },
  ...(
    [
      ['Platform Engineers', 'Manage collectors, pipelines and integrations.'],
      ['Storefront Developers', 'Edit storefront dashboards and monitors.'],
      ['Checkout Developers', 'Edit checkout dashboards and monitors.'],
      ['Billing Developers', 'Edit billing dashboards and monitors.'],
      ['Data Analysts', 'Read telemetry and export dashboards.'],
      ['Support Read Access', 'Read logs and traces without sensitive attributes.'],
    ] as const
  ).map(([name, description]) => ({ id: `${org}-${name.toLowerCase().replace(/ /g, '-')}`, name, description, managed: false })),
];
const CUSTOM_ROLE_IDS = ROLES.filter((x) => !x.managed).map((x) => x.id);

const TITLES = ['Software Engineer', 'Senior Software Engineer', 'Site Reliability Engineer', 'Engineering Manager', 'Data Engineer', 'QA Engineer', 'Product Manager', 'Support Engineer'];
const ZONES = ['Europe/Berlin', 'Europe/London', 'America/New_York', 'Asia/Jerusalem', 'Europe/Warsaw'];
/** The first five are the people of the other mocks (error tracking assignees, CI authors). */
const NAMES = [
  'Alex Morgan',
  'Priya Shah',
  'Daniel Kim',
  'Sara Levi',
  'Omar Haddad',
  'Jordan Baker',
  'Taylor Nakamura',
  'Casey Ortiz',
  'Riley Jensen',
  'Avery Patel',
  'Quinn Rossi',
  'Jamie Walsh',
  'Rowan Silva',
  'Skyler Meyer',
  'Emerson Okafor',
  'Hayden Xu',
  'Parker Lindqvist',
  'Reese Kowalski',
  'Sawyer Turner',
  'Dakota Foster',
  'Elliot Garcia',
  'Finley Novak',
  'Harper Ito',
  'Kendall Young',
  'Logan Diaz',
  'Micah Evans',
];

function seedUsers(now: number): UserRow[] {
  return NAMES.map((name, i) => {
    const me = i === 0;
    // Everyone signs in; a few newer people haven't accepted their invitation and one left.
    const status: S['UserStatus'] = me || i < 20 ? 'active' : i < 24 ? 'pending' : 'deactivated';
    const login = status === 'pending' ? [] : me ? ['saml'] : pick([['saml'], ['saml'], ['saml'], ['google'], ['password', 'google'], ['password']]);
    const base = me || i === 1 || i === 7 ? `${org}-admin` : i % 6 === 5 ? `${org}-readonly` : `${org}-standard`;
    const custom = CUSTOM_ROLE_IDS.filter(() => r() < 0.2);
    return {
      id: me ? ME_ID : uuid(),
      name,
      email: `${name.toLowerCase().replace(' ', '.')}@example.com`,
      title: me ? 'Platform Engineer' : pick(TITLES),
      status,
      loginMethods: login,
      mfa: me || (status !== 'pending' && r() < 0.35),
      timezone: me ? 'Europe/Berlin' : pick(ZONES),
      lastLoginAt: status === 'active' ? now - (me ? 2 * 3600_000 : Math.round(r() * 30 * DAY)) : null,
      createdAt: now - (30 + Math.floor(r() * 700)) * DAY,
      roleIds: [base, ...(me ? [`${org}-platform-engineers`] : custom)],
    };
  });
}

const TEAM_DEFS: [handle: string, name: string, description: string, channel: boolean][] = [
  ['platform', 'Platform', 'Shared infrastructure, CI/CD and the API gateway.', true],
  ['storefront', 'Storefront', 'Customer-facing web storefront and storefront-api.', true],
  ['checkout', 'Checkout', 'Cart, checkout, order-service and inventory reservation.', true],
  ['billing', 'Billing', 'Invoices, the billing service and payment reconciliation.', true],
  ['payments', 'Payments', 'Payment providers and fraud checks.', false],
  ['catalog', 'Catalog', 'Product catalog, catalog-service, pricing and stock levels.', true],
  ['data-platform', 'Data Platform', 'Pipelines, warehouse and reporting.', false],
  ['mobile', 'Mobile', 'iOS and Android shop apps.', false],
  ['sre', 'SRE', 'Reliability, on-call tooling and incident process.', true],
  ['security', 'Security', 'Application and cloud security.', false],
  ['notifications', 'Notifications', 'Email, push and the notification worker.', true],
  ['identity', 'Identity', 'Login, accounts and SSO.', false],
  ['search', 'Search', 'Product search and recommendations.', false],
];

function seedTeams(users: UserRow[]): TeamRow[] {
  const others = users.filter((u) => u.id !== ME_ID && u.status === 'active');
  return TEAM_DEFS.map(([handle, name, description, channel]) => {
    const size = 2 + Math.floor(r() * 7);
    const members: TeamRow['members'] = [];
    while (members.length < size) {
      const u = pick(others);
      if (!members.some((m) => m.userId === u.id)) members.push({ userId: u.id, role: members.length === 0 ? 'admin' : 'member' });
    }
    if (handle === 'platform' || handle === 'sre') members.push({ userId: ME_ID, role: 'admin' });
    return {
      handle,
      name,
      description,
      restricted: r() < 0.6,
      createdVia: pick(['ui', 'ui', 'saml', 'api', 'terraform']),
      notifications: channel ? [`#${handle}-alerts`] : [],
      parent: handle === 'payments' ? 'billing' : handle === 'sre' || handle === 'data-platform' ? 'platform' : null,
      members,
    };
  });
}

interface KeyRow {
  id: string;
  name: string;
  prefix: string;
  createdBy: string | null;
  createdAt: number;
  lastUsedAt: number | null;
  /** Ingest keys: set once revoked (they stay listed). */
  revokedAt?: number | null;
}

interface State {
  users: UserRow[];
  roles: RoleRow[];
  teams: TeamRow[];
  mappings: (S['SamlMapping'] & { kind: S['MappingKind'] })[];
  ingestKeys: KeyRow[];
  appKeys: KeyRow[];
  sessions: S['SessionInfo'][];
  audit: S['AuditEvent'][];
  login: S['AuthSettingsView'];
  hasPassword: boolean;
  mfaPending: string | null;
  seq: number;
}

let state: State | null = null;

function db(): State {
  if (state) return state;
  const now = Date.now();
  const users = seedUsers(now);
  const teams = seedTeams(users);
  const name = (i: number) => users[i].name;
  let seq = 1;
  const mappings: State['mappings'] = [
    ...teams.slice(0, 8).map((t, i) => ({ kind: 'team' as const, id: seq++, target: t.handle, targetName: t.handle, attributeKey: CLAIM, attributeValue: uuid(), createdAt: now - (5 + i * 7) * DAY })),
    ...[`${org}-admin`, `${org}-standard`, ...CUSTOM_ROLE_IDS.slice(0, 3)].map((id, i) => ({
      kind: 'role' as const,
      id: seq++,
      target: id,
      targetName: ROLES.find((x) => x.id === id)!.name,
      attributeKey: CLAIM,
      attributeValue: uuid(),
      createdAt: now - (9 + i * 11) * DAY,
    })),
  ];
  const key = (name: string, prefix: string, createdBy: string | null, ageDays: number, usedHours: number | null): KeyRow => ({
    id: uuid(),
    name,
    prefix: token(prefix).slice(0, 10),
    createdBy,
    createdAt: now - ageDays * DAY,
    lastUsedAt: usedHours === null ? null : now - usedHours * 3600_000,
  });
  const ingestKeys = [
    key('datadog-agent-eks', 'ocik_', users[1].name, 1, 0.01),
    key('prod-otel-collector', 'ocik_', users[0].name, 45, 0.02),
    key('staging-otel-collector', 'ocik_', users[7].name, 90, 30),
    key('Local demo generators', 'ocik_', null, 120, 0.01),
    // Last used the day before it was revoked.
    { ...key('legacy-fluentd', 'ocik_', users[2].name, 200, 151 * 24), revokedAt: Date.now() - 150 * 86_400_000 },
  ];
  const appKeys = [key('terraform-dashboards', 'ocak_', null, 17, 5), key('ci-export', 'ocak_', null, 63, null)];
  const audit: [actor: string, action: string, target: string, detail: string, hoursAgo: number][] = [
    [name(0), 'login.succeeded', '', 'saml', 2],
    [name(1), 'ingest_key.created', ingestKeys[0].prefix, 'datadog-agent-eks', 24],
    [name(0), 'user.invited', users[23].email, '', 50],
    [name(7), 'user.roles_changed', users[11].email, `${org}-standard, ${org}-data-analysts`, 75],
    [name(0), 'org.group_mapping_created', 'checkout', `${CLAIM} = ${mappings[2].attributeValue}`, 120],
    [name(1), 'user.deactivated', users[25].email, '', 160],
    [name(0), 'app_key.created', appKeys[0].prefix, 'terraform-dashboards', 17 * 24],
    [name(0), 'org.login_methods_updated', '', 'saml=true', 900],
  ];
  state = {
    users,
    roles: ROLES,
    teams,
    mappings,
    ingestKeys,
    appKeys,
    sessions: [
      { id: hex(64), method: 'saml', ip: '203.0.113.24', userAgent: navigatorAgent(), createdAt: now - 2 * 3600_000, lastSeenAt: now - 60_000, current: true, orgId: org },
      {
        id: hex(64),
        method: 'saml',
        ip: '198.51.100.7',
        userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148',
        createdAt: now - 3 * DAY,
        lastSeenAt: now - 20 * 3600_000,
        current: false,
        orgId: org,
      },
    ],
    audit: audit.map(([actor, action, target, detail, h], i) => ({ id: audit.length - i, actor, action, target, detail, ip: '203.0.113.24', createdAt: now - h * 3600_000 })),
    login: {
      passwordEnabled: true,
      ssoRequired: false,
      jitProvisioning: true,
      defaultRoleId: `${org}-standard`,
      saml: {
        enabled: true,
        idpMetadata: '<EntityDescriptor xmlns="urn:oasis:names:tc:SAML:2.0:metadata" entityID="https://sts.example.com/6f1d2c3b/"><!-- example IdP --></EntityDescriptor>',
        idpEntityId: 'https://sts.example.com/6f1d2c3b/',
        idpSsoUrl: 'https://login.example.com/6f1d2c3b/saml2',
        problem: null,
        emailAttribute: 'http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress',
        nameAttribute: 'http://schemas.microsoft.com/identity/claims/displayname',
        groupsAttribute: CLAIM,
      },
      oidc: { enabled: false, issuer: null, clientId: null, hasClientSecret: false, groupsClaim: 'groups', buttonLabel: 'Sign in with SSO' },
      serviceProvider: serviceProvider(),
    },
    hasPassword: true,
    mfaPending: null,
    seq: 100,
  };
  return state;
}

function navigatorAgent(): string {
  return typeof navigator !== 'undefined' ? navigator.userAgent : 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/537.36 Chrome/130.0 Safari/537.36';
}

function serviceProvider(): S['ServiceProviderInfo'] {
  const base = `${origin()}/api/v1/auth/saml/${org}`;
  return { entityId: `${base}/metadata`, metadataUrl: `${base}/metadata`, acsUrl: `${base}/acs`, oidcRedirectUrl: `${origin()}/api/v1/auth/oidc/${org}/callback` };
}

function me(): UserRow {
  return db().users.find((u) => u.id === ME_ID)!;
}

function audit(action: string, target: string, detail: string): void {
  const s = db();
  s.audit.unshift({ id: (s.audit[0]?.id ?? 0) + 1, actor: me().name, action, target, detail, ip: '203.0.113.24', createdAt: Date.now() });
}

// ================================================================ shared with the incidents mock

/** Directory lookups for other mocks (incident commanders, responders, paged teams). */
export function mockUser(id: string): { id: string; name: string; status: S['UserStatus'] } | undefined {
  return db().users.find((u) => u.id === id);
}
export function mockUserIds(): string[] {
  return db()
    .users.filter((u) => u.status === 'active')
    .map((u) => u.id);
}
export function mockTeam(handle: string): { handle: string; notifications: string[] } | undefined {
  return db().teams.find((t) => t.handle === handle);
}
export const mockMe = () => ({ id: ME_ID, name: me().name });

// ================================================================ profile, users, roles

function summary(u: UserRow): S['UserSummary'] {
  const roles = u.roleIds
    .map((id) => db().roles.find((x) => x.id === id))
    .filter((x): x is RoleRow => !!x)
    .sort((a, b) => Number(b.managed) - Number(a.managed) || a.name.localeCompare(b.name))
    .map(({ id, name }) => ({ id, name }));
  return { id: u.id, name: u.name, email: u.email, title: u.title, status: u.status, roles, loginMethods: [...u.loginMethods], mfa: u.mfa, lastLoginAt: u.lastLoginAt, createdAt: u.createdAt };
}

export async function getProfile(): Promise<S['Profile']> {
  const u = me();
  const teams = db()
    .teams.filter((t) => t.members.some((m) => m.userId === ME_ID))
    .sort(byName)
    .map(({ handle, name }) => ({ handle, name }));
  return { ...summary(u), timezone: u.timezone, org: { ...MOCK_ORG }, teams, isInstanceAdmin: true };
}

function checkName(s: string, what: string, max: number): string {
  const t = s.trim();
  if (!t || [...t].length > max) fail(`${what} must be 1–${max} characters`);
  return t;
}

export async function updateProfile(p: S['ProfilePatch']): Promise<S['Profile']> {
  const u = me();
  if (p.name != null) u.name = checkName(p.name, 'name', 120);
  if (p.title != null) u.title = [...p.title.trim()].slice(0, 120).join('');
  if (p.timezone != null) u.timezone = p.timezone;
  return getProfile();
}

export async function searchUsers(req: S['UserSearchRequest']): Promise<S['UserSearchPage']> {
  const text = (req.query ?? '').trim();
  const mfa = req.mfa ?? [];
  const [cfg, uncfg] = [mfa.includes('configured'), mfa.includes('unconfigured')];
  const all = db()
    .users.filter(
      (u) =>
        (!text || ilike(u.name, text) || ilike(u.email, text)) &&
        (!req.statuses?.length || req.statuses.includes(u.status)) &&
        (!req.loginMethods?.length || u.loginMethods.some((m) => req.loginMethods!.includes(m))) &&
        (cfg === uncfg || u.mfa === cfg) &&
        (!req.roleIds?.length || u.roleIds.some((x) => req.roleIds!.includes(x))),
    )
    .sort((a, b) => byName(a, b) || a.id.localeCompare(b.id));
  const offset = Math.max(0, req.offset);
  return { total: all.length, users: all.slice(offset, offset + clamp(req.limit)).map(summary) };
}

const linkUrl = () => `${origin()}/login/link/${token('')}`;

export async function inviteUsers(emails: string[], roleIds: string[]): Promise<S['Invitation'][]> {
  if (emails.length === 0 || emails.length > 50) fail('invite 1–50 emails at a time');
  const s = db();
  const fresh: UserRow[] = [];
  for (const raw of emails) {
    const email = raw.trim().toLowerCase();
    const [local, domain] = [email.slice(0, email.indexOf('@')), email.slice(email.indexOf('@') + 1)];
    if (email.length > 254 || !email.includes('@') || !local || !domain.includes('.') || domain.startsWith('.')) fail(`'${raw}' is not a valid email address`);
    if (s.users.some((u) => u.email === email) || fresh.some((u) => u.email === email)) continue;
    fresh.push({
      id: uuid(),
      email,
      name: local.replace(/[._]/g, ' '),
      title: null,
      status: 'pending',
      loginMethods: [],
      mfa: false,
      timezone: null,
      lastLoginAt: null,
      createdAt: Date.now(),
      roleIds: roleIds.filter((id) => s.roles.some((x) => x.id === id)),
    });
  }
  s.users.push(...fresh);
  return fresh.map((u) => {
    audit('user.invited', u.email, '');
    return { id: u.id, email: u.email, url: linkUrl() };
  });
}

export async function updateUser(id: string, p: S['UserPatch']): Promise<void> {
  if (id === ME_ID && p.status === 'deactivated') fail("you can't deactivate yourself");
  const u = db().users.find((x) => x.id === id) ?? fail(`User ${id} not found`);
  if (id === ME_ID && p.roleIds && !p.roleIds.includes(`${org}-admin`)) fail("you can't remove your own admin role");
  if (p.status) {
    u.status = p.status;
    audit(`user.${p.status}`, u.email, '');
  }
  if (p.roleIds) {
    u.roleIds = p.roleIds.filter((x) => db().roles.some((role) => role.id === x));
    audit('user.roles_changed', u.email, p.roleIds.join(', '));
  }
}

export async function loginLink(userId: string): Promise<S['LoginLink']> {
  const u = db().users.find((x) => x.id === userId) ?? fail(`User ${userId} not found`);
  if (u.status === 'deactivated') fail('Reactivate the user first');
  audit('user.login_link_issued', u.email, u.status === 'pending' ? 'invite' : 'reset');
  return { url: linkUrl() };
}

export async function listRoles(): Promise<S['Role'][]> {
  const s = db();
  return [...s.roles].sort((a, b) => Number(b.managed) - Number(a.managed) || a.name.localeCompare(b.name)).map((x) => ({ ...x, userCount: s.users.filter((u) => u.roleIds.includes(x.id)).length }));
}

// ================================================================ teams

export async function searchTeams(req: S['TeamSearchRequest']): Promise<S['TeamSearchPage']> {
  const s = db();
  const text = (req.query ?? '').trim();
  const member = req.member === 'me' ? ME_ID : req.member;
  const all = s.teams
    .filter(
      (t) =>
        (!text || ilike(t.name, text) || ilike(t.handle, text) || ilike(t.description, text)) &&
        (member == null || t.members.some((m) => m.userId === member)) &&
        (req.restricted == null || t.restricted === req.restricted) &&
        (req.createdVia == null || t.createdVia === req.createdVia) &&
        (req.hasNotifications == null || t.notifications.length > 0 === req.hasNotifications),
    )
    .sort((a, b) => (req.sort === 'members' ? b.members.length - a.members.length : 0) || byName(a, b));
  const offset = Math.max(0, req.offset);
  const teams = all.slice(offset, offset + clamp(req.limit)).map((t) => ({
    handle: t.handle,
    name: t.name,
    description: t.description,
    restricted: t.restricted,
    createdVia: t.createdVia,
    notifications: [...t.notifications],
    memberCount: t.members.length,
    members: t.members
      .map((m) => ({ role: m.role, user: s.users.find((u) => u.id === m.userId)! }))
      .sort((a, b) => a.role.localeCompare(b.role) || byName(a.user, b.user))
      .slice(0, 3)
      .map(({ user }) => ({ id: user.id, name: user.name })),
    subteams: s.teams
      .filter((c) => c.parent === t.handle)
      .map((c) => c.handle)
      .sort(),
  }));
  return { teams, total: all.length };
}

export async function createTeam(req: S['CreateTeamRequest']): Promise<void> {
  const handle = req.handle.trim().toLowerCase();
  if (!handle || handle.length > 64 || !/^[a-z0-9_-]+$/.test(handle)) fail("handle must be 1–64 characters: letters, digits, '-' or '_'");
  const name = checkName(req.name, 'name', 120);
  const s = db();
  if (s.teams.some((t) => t.handle === handle)) fail(`a team with handle '${handle}' already exists`);
  s.teams.push({ handle, name, description: (req.description ?? '').trim(), restricted: false, createdVia: 'ui', notifications: [], parent: null, members: [{ userId: ME_ID, role: 'admin' }] });
}

// ================================================================ SAML group mappings

export async function listMappings(kind: S['MappingKind']): Promise<S['SamlMapping'][]> {
  return db()
    .mappings.filter((m) => m.kind === kind)
    .sort((a, b) => a.createdAt - b.createdAt)
    .map(({ kind: _kind, ...m }) => m);
}

export async function createMapping(req: S['CreateMappingRequest']): Promise<void> {
  const key = checkName(req.attributeKey, 'attribute key', 500);
  const value = checkName(req.attributeValue, 'attribute value', 500);
  const s = db();
  const targetName = req.kind === 'team' ? s.teams.find((t) => t.handle === req.target)?.handle : s.roles.find((x) => x.id === req.target)?.name;
  if (!targetName) fail(`unknown ${req.kind} '${req.target}'`);
  s.mappings.push({ kind: req.kind, id: s.seq++, target: req.target, targetName: targetName!, attributeKey: key, attributeValue: value, createdAt: Date.now() });
  audit('org.group_mapping_created', req.target, `${key} = ${value}`);
}

export async function deleteMapping(id: number): Promise<void> {
  const s = db();
  const at = s.mappings.findIndex((m) => m.id === id);
  if (at < 0) fail(`Mapping ${id} not found`);
  s.mappings.splice(at, 1);
  audit('org.group_mapping_deleted', String(id), '');
}

// ================================================================ sign-in settings, sessions, MFA

export async function getLoginMethods(): Promise<S['AuthSettingsView']> {
  return structuredClone({ ...db().login, serviceProvider: serviceProvider() });
}

const opt = (v?: string | null) => (v ?? '').trim() || null;

export async function updateLoginMethods(u: S['AuthSettingsUpdate']): Promise<S['AuthSettingsView']> {
  const s = db();
  const cur = s.login;
  const metaUrl = opt(u.samlIdpMetadataUrl);
  // The mock can't fetch the URL; it stands in a document naming it.
  const metadata = metaUrl ? `<EntityDescriptor xmlns="urn:oasis:names:tc:SAML:2.0:metadata" entityID="${metaUrl}"/>` : (opt(u.samlIdpMetadata) ?? cur.saml.idpMetadata ?? null);
  if (u.samlEnabled && !metadata) fail('Provide the IdP metadata before enabling SAML');
  const issuer = opt(u.oidcIssuer)?.replace(/\/+$/, '') ?? null;
  const clientId = opt(u.oidcClientId);
  if (u.oidcEnabled && (!issuer || !clientId)) fail('OIDC needs an issuer URL and a client ID');
  if (!u.passwordEnabled && !u.samlEnabled && !u.oidcEnabled) fail('Enable at least one login method');
  if (u.ssoRequired && !u.samlEnabled && !u.oidcEnabled) fail('Enable SAML or OIDC before requiring SSO');
  const changes = (
    [
      ['password', cur.passwordEnabled, u.passwordEnabled],
      ['sso_required', cur.ssoRequired, u.ssoRequired],
      ['saml', cur.saml.enabled, u.samlEnabled],
      ['oidc', cur.oidc.enabled, u.oidcEnabled],
      ['jit', cur.jitProvisioning, u.jitProvisioning],
    ] as const
  )
    .filter(([, a, b]) => a !== b)
    .map(([k, , b]) => `${k}=${b}`)
    .join(', ');
  const entity = metadata?.match(/entityID="([^"]+)"/)?.[1] ?? null;
  s.login = {
    passwordEnabled: u.passwordEnabled,
    ssoRequired: u.ssoRequired,
    jitProvisioning: u.jitProvisioning,
    defaultRoleId: s.roles.some((x) => x.id === u.defaultRoleId) ? u.defaultRoleId : null,
    saml: {
      enabled: u.samlEnabled,
      idpMetadata: metadata,
      idpEntityId: entity,
      idpSsoUrl: metadata === cur.saml.idpMetadata ? cur.saml.idpSsoUrl : null,
      problem: null,
      emailAttribute: opt(u.samlEmailAttribute) ?? cur.saml.emailAttribute,
      nameAttribute: opt(u.samlNameAttribute) ?? cur.saml.nameAttribute,
      groupsAttribute: opt(u.samlGroupsAttribute) ?? cur.saml.groupsAttribute,
    },
    oidc: {
      enabled: u.oidcEnabled,
      issuer,
      clientId,
      hasClientSecret: cur.oidc.hasClientSecret || !!opt(u.oidcClientSecret),
      groupsClaim: opt(u.oidcGroupsClaim) ?? cur.oidc.groupsClaim,
      buttonLabel: opt(u.oidcButtonLabel) ?? cur.oidc.buttonLabel,
    },
    serviceProvider: serviceProvider(),
  };
  audit('org.login_methods_updated', '', changes);
  return getLoginMethods();
}

export async function auditTrail(query: string, offset: number, limit: number): Promise<S['AuditPage']> {
  const q = query.trim();
  const all = db().audit.filter((e) => !q || [e.actor, e.action, e.target, e.detail].some((v) => ilike(v, q)));
  const from = Math.max(0, offset);
  return { events: all.slice(from, from + clamp(limit)), total: all.length };
}

export async function securityStatus(): Promise<S['SecurityStatus']> {
  return { hasPassword: db().hasPassword, mfaEnabled: me().mfa, access: me().roleIds.includes(`${org}-admin`) ? 'admin' : 'standard' };
}

export async function changePassword(newPassword: string, currentPassword?: string): Promise<void> {
  const s = db();
  if (s.hasPassword && !currentPassword) fail('Current password is incorrect');
  const n = [...newPassword].length;
  if (n < 12) fail('password must be at least 12 characters');
  if (n > 128) fail('password must be at most 128 characters');
  if ([...newPassword].every((c) => c === newPassword[0])) fail('password must not be a single repeated character');
  s.hasPassword = true;
  if (!me().loginMethods.includes('password')) me().loginMethods.push('password');
  s.sessions = s.sessions.filter((x) => x.current);
  audit('user.password_changed', '', '');
}

export async function listSessions(): Promise<S['SessionInfo'][]> {
  return [...db().sessions].sort((a, b) => b.lastSeenAt - a.lastSeenAt);
}

export async function revokeSession(id: string): Promise<void> {
  const s = db();
  if (!s.sessions.some((x) => x.id === id)) fail('Session not found');
  s.sessions = s.sessions.filter((x) => x.id !== id);
  audit('session.revoked', id.slice(0, 12), '');
}

const B32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';

export async function mfaSetup(): Promise<S['MfaSetup']> {
  if (me().mfa) fail('MFA is already enabled. Disable it first to enroll a new device.');
  const secret = Array.from({ length: 32 }, () => B32[Math.floor(r() * 32)]).join('');
  db().mfaPending = secret;
  return { secret, otpauthUrl: `otpauth://totp/ObserveCraft:${encodeURIComponent(me().email)}?secret=${secret}&issuer=ObserveCraft&digits=6&period=30` };
}

/** Mock mode accepts any six-digit code. */
const checkCode = (code: string, message: string) => /^\d{6}$/.test(code) || fail(message);

export async function mfaEnable(code: string): Promise<void> {
  if (!db().mfaPending) fail('Start MFA setup first');
  checkCode(code, "That code doesn't match. Check the time on your device and try again.");
  db().mfaPending = null;
  me().mfa = true;
  audit('user.mfa_enabled', '', '');
}

export async function mfaDisable(code: string): Promise<void> {
  if (!me().mfa) fail('MFA is not enabled');
  checkCode(code, 'Invalid code');
  // Memberships signed in through an IdP keep the IdP's MFA flag.
  if (me().loginMethods.includes('password')) me().mfa = false;
  audit('user.mfa_disabled', '', '');
}

export async function myOrgs(): Promise<S['MyOrg'][]> {
  return [{ id: MOCK_ORG.id, name: MOCK_ORG.name, current: true, isAdmin: true }];
}

// ================================================================ application and ingest keys

const keyInfo = ({ id, name, prefix, createdAt, lastUsedAt }: KeyRow) => ({ id, name, prefix, createdAt, lastUsedAt });

function checkKeyName(name: string): string {
  const n = name.trim();
  if (!n || [...n].length > 100) fail('name must be 1–100 characters');
  return n;
}

export async function listAppKeys(): Promise<S['AppKey'][]> {
  return [...db().appKeys].sort((a, b) => b.createdAt - a.createdAt).map(keyInfo);
}

export async function createAppKey(name: string): Promise<S['CreatedAppKey']> {
  const s = db();
  const n = checkKeyName(name);
  if (s.appKeys.length >= 25) fail('You already have 25 active keys. Revoke one first.');
  const key = token('ocak_');
  const row: KeyRow = { id: uuid(), name: n, prefix: key.slice(0, 10), createdBy: null, createdAt: Date.now(), lastUsedAt: null };
  s.appKeys.push(row);
  audit('app_key.created', row.prefix, n);
  return { ...keyInfo(row), key };
}

export async function revokeAppKey(id: string): Promise<void> {
  const s = db();
  const row = s.appKeys.find((k) => k.id === id) ?? fail('Key not found');
  s.appKeys = s.appKeys.filter((k) => k !== row);
  audit('app_key.revoked', row.prefix, '');
}

export async function listIngestKeys(): Promise<S['IngestKey'][]> {
  return [...db().ingestKeys]
    .sort((a, b) => Number(Boolean(a.revokedAt)) - Number(Boolean(b.revokedAt)) || b.createdAt - a.createdAt)
    .map((k) => ({ ...keyInfo(k), createdBy: k.createdBy, revokedAt: k.revokedAt ?? null }));
}

export async function createIngestKey(name: string): Promise<S['CreatedIngestKey']> {
  const s = db();
  const n = checkKeyName(name);
  if (s.ingestKeys.filter((k) => !k.revokedAt).length >= 50) fail('This organization already has 50 active keys. Revoke one first.');
  const key = token('ocik_');
  const row: KeyRow = { id: uuid(), name: n, prefix: key.slice(0, 10), createdBy: me().name, createdAt: Date.now(), lastUsedAt: null };
  s.ingestKeys.push(row);
  audit('ingest_key.created', row.prefix, n);
  return { ...keyInfo(row), createdBy: row.createdBy, key };
}

export async function revokeIngestKey(id: string): Promise<void> {
  const s = db();
  const row = s.ingestKeys.find((k) => k.id === id && !k.revokedAt) ?? fail('Key not found');
  row.revokedAt = Date.now();
  audit('ingest_key.revoked', row.prefix, row.name);
}
