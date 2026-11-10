// In-browser stand-in for an Azure app registration's Entra ID logs (routes/entra_logs.rs): the settings live in
// memory for the session (the demo app registration has them on); the test reads sign-ins and audits and finds no
// Entra ID P2 for risk detections. `entraLogs` is the mock tenant's logs (`source:azure.activedirectory`), shaped as
// crate::entra::logs makes them (Azure's diagnostic settings export) and the integration's pipeline completes them
// (`evt.*`, `usr.*`, `network.client.ip`, `title`).
import type { EntraLogs, EntraLogsInput, EntraLogsTest } from '../azure/types';
import type { JsonValue, LogEvent } from '../types';
import type { Rand } from './generator';

const MIN = 60_000;
const HOUR = 60 * MIN;
const TENANT = '7d3e5f1a-2b4c-4d6e-8f0a-1b2c3d4e5f60';

const delay = <T>(value: T, ms = 150) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));

/** Settings by app registration; the demo one (mock/azure.ts) has sign-ins and audits on. */
const settings = new Map<string, { signIns: boolean; audits: boolean; risk: boolean; at: number }>([
  ['3c2b1a09-8f7e-4d6c-9b5a-4f3e2d1c0b9a', { signIns: true, audits: true, risk: false, at: Date.now() - 3 * 86_400_000 }],
]);

function view(id: string): EntraLogs {
  const s = settings.get(id);
  if (!s) return { enabled: false, signIns: true, audits: true, risk: false, streams: [] };
  const now = Date.now();
  const run = now - ((now - s.at) % (3 * MIN));
  const scopes = [s.audits && 'audits', s.risk && 'risk', s.signIns && 'signins'].filter((x): x is string => !!x);
  return {
    enabled: true,
    signIns: s.signIns,
    audits: s.audits,
    risk: s.risk,
    streams: scopes.map((scope) =>
      scope === 'risk'
        ? {
            scope,
            last_success_at: null,
            collected_until: null,
            metrics: 0,
            points: 0,
            error: {
              kind: 'access_denied',
              message:
                'Reading risk detections needs Microsoft Entra ID P2 on the tenant. Graph: Forbidden: Your tenant is not licensed for this feature. Learn more about the license requirements for Identity Protection.',
              missing_action: null,
              at: run,
            },
          }
        : { scope, last_success_at: run, collected_until: run - MIN, metrics: scope === 'signins' ? 41 : 6, points: scope === 'signins' ? 41 : 6, error: null },
    ),
  };
}

/** Whether an app registration has Entra ID logs on: the Microsoft Entra ID tile is installed, as the server counts it. */
export const entraConnected = (): boolean => settings.size > 0;

/** An app registration removed (mock/azure.ts): its settings go too. */
export const forgetEntraLogs = (id: string): void => void settings.delete(id);

export const getEntraLogs = (id: string): Promise<EntraLogs> => delay(view(id));

export function saveEntraLogs(id: string, input: EntraLogsInput): Promise<EntraLogs> {
  const on = { signIns: input.signIns ?? true, audits: input.audits ?? true, risk: input.risk ?? false };
  if ((input.enabled ?? true) && (on.signIns || on.audits || on.risk)) settings.set(id, { ...on, at: Date.now() });
  else settings.delete(id);
  return delay(view(id), 250);
}

export function testEntraLogs(): Promise<EntraLogsTest> {
  return delay(
    {
      ok: false,
      streams: [
        { stream: 'signins', ok: true, message: 'Can read the sign-in logs: 812 in the last hour', permission: 'AuditLog.Read.All', license: 'Microsoft Entra ID P1 or P2' },
        { stream: 'audits', ok: true, message: 'Can read the audit logs: 37 in the last hour', permission: 'AuditLog.Read.All', license: null },
        {
          stream: 'risk',
          ok: false,
          message: 'Reading risk detections needs Microsoft Entra ID P2 on the tenant. Graph: Forbidden: Your tenant is not licensed for this feature.',
          permission: 'IdentityRiskEvent.Read.All',
          license: 'Microsoft Entra ID P2',
        },
      ],
    },
    500,
  );
}

// ---------------------------------------------------------------- the tenant's logs

/** [upn, display name, object id, home country]. */
const PEOPLE: [string, string, string, string][] = [
  ['jordan.patel@example.com', 'Jordan Patel', '0f3a7c52-6d1e-4b7a-9c2d-5e8f1a2b3c4d', 'RO'],
  ['sam.lee@example.com', 'Sam Lee', '9ff6ab06-49d6-49e8-9974-e979522283c1', 'US'],
  ['riley.chen@example.com', 'Riley Chen', 'febf6383-8bed-4efb-8244-c9a456a1f172', 'DE'],
  ['alex.morgan@example.com', 'Alex Morgan', '2b4c6d8e-0f1a-4b3c-8d5e-6f7a8b9c0d1e', 'US'],
  ['taylor.kim@example.com', 'Taylor Kim', '3c5d7e9f-1a2b-4c4d-9e6f-7a8b9c0d1e2f', 'NL'],
  ['casey.nguyen@example.com', 'Casey Nguyen', '4d6e8f0a-2b3c-4d5e-8f7a-8b9c0d1e2f3a', 'RO'],
  ['morgan.diaz@example.com', 'Morgan Diaz', '5e7f9a1b-3c4d-4e6f-9a8b-9c0d1e2f3a4b', 'BR'],
];
const ADMINS = PEOPLE.slice(0, 2);
const APPS = ['Azure Portal', 'Microsoft Teams', 'Office 365 Exchange Online', 'Microsoft Azure CLI', 'Storefront Admin', 'ObserveCraft'];
const PLACES: Record<string, [city: string, state: string, lat: number, lon: number, ip: string]> = {
  RO: ['Bucharest', 'Bucuresti', 44.43, 26.1, '203.0.113.24'],
  US: ['Seattle', 'Washington', 47.61, -122.33, '198.51.100.7'],
  DE: ['Berlin', 'Berlin', 52.52, 13.4, '203.0.113.88'],
  NL: ['Amsterdam', 'Noord-Holland', 52.37, 4.9, '192.0.2.45'],
  BR: ['Sao Paulo', 'Sao Paulo', -23.55, -46.63, '198.51.100.140'],
  NG: ['Lagos', 'Lagos', 6.45, 3.39, '198.51.100.200'],
};
/** [error code, failure reason, share of failures]. */
const FAILURES: [number, string, number][] = [
  [50126, 'Error validating credentials due to invalid username or password.', 0.55],
  [50074, 'Strong Authentication is required.', 0.2],
  [50053, 'Account is locked because user tried to sign in too many times with an incorrect user ID or password.', 0.1],
  [53003, 'Access has been blocked by Conditional Access policies.', 0.1],
  [50057, 'User account is disabled.', 0.05],
];
const DEVICES: [string, string][] = [
  ['Windows10', 'Edge 130.0.0'],
  ['MacOs', 'Chrome 131.0.0'],
  ['Ios', 'Mobile Safari'],
  ['Android', 'Chrome Mobile 131'],
  ['Linux', 'Firefox 133.0'],
];

const pick = <T>(r: Rand, xs: T[]): T => xs[Math.floor(r() * xs.length)];
const uuid = (r: Rand) => '10000000-1000-4000-8000-100000000000'.replace(/[018]/g, (c) => (Number(c) ^ (Math.floor(r() * 16) >> (Number(c) / 4))).toString(16));
const iso = (ms: number) => new Date(ms).toISOString();
/** One of `xs`, each as likely as its share (its last element). */
function weighted<T extends [...unknown[], number]>(r: Rand, xs: T[]): T {
  let left = r();
  for (const x of xs) {
    left -= x[x.length - 1] as number;
    if (left < 0) return x;
  }
  return xs[0];
}

function base(category: string, operation: string, at: number, ip: string | null, correlation: string): Record<string, JsonValue> {
  const out: Record<string, JsonValue> = {
    time: iso(at),
    resourceId: `/tenants/${TENANT}/providers/Microsoft.aadiam`,
    operationVersion: '1.0',
    tenantId: TENANT,
    resultSignature: 'None',
    durationMs: 0,
    duration: 0,
    Level: 4,
    correlationId: correlation,
    category,
    operationName: operation,
  };
  if (ip) out.callerIpAddress = ip;
  return out;
}

function signIn(r: Rand, i: number, at: number): LogEvent {
  const person = r() < 0.3 ? PEOPLE[0] : pick(r, PEOPLE);
  // Some sign-ins come from abroad: an anonymous address in Nigeria, rated risky.
  const abroad = r() < 0.04;
  const country = abroad
    ? 'NG'
    : r() < 0.85
      ? person[3]
      : pick(
          r,
          Object.keys(PLACES).filter((c) => c !== 'NG'),
        );
  const [city, state, lat, lon, ip] = PLACES[country];
  const failed = abroad ? r() < 0.7 : r() < 0.13;
  const failure = failed ? weighted(r, FAILURES) : null;
  const app = pick(r, APPS);
  const [os, browser] = pick(r, DEVICES);
  const risk = abroad ? (r() < 0.6 ? 'high' : 'medium') : r() < 0.02 ? 'low' : 'none';
  const correlation = uuid(r);
  const properties: Record<string, JsonValue> = {
    id: uuid(r),
    createdDateTime: iso(at),
    userDisplayName: person[1],
    userPrincipalName: person[0],
    userId: person[2],
    appDisplayName: app,
    appId: uuid(r),
    ipAddress: ip,
    clientAppUsed: app === 'Microsoft Azure CLI' ? 'Mobile Apps and Desktop clients' : r() < 0.8 ? 'Browser' : 'Mobile Apps and Desktop clients',
    correlationId: correlation,
    conditionalAccessStatus: failure?.[0] === 53003 ? 'failure' : r() < 0.7 ? 'success' : 'notApplied',
    isInteractive: true,
    authenticationRequirement: r() < 0.82 ? 'multiFactorAuthentication' : 'singleFactorAuthentication',
    riskDetail: 'none',
    riskLevelAggregated: risk,
    riskLevelDuringSignIn: risk,
    riskState: risk === 'none' ? 'none' : 'atRisk',
    resourceDisplayName: app === 'Azure Portal' ? 'Windows Azure Service Management API' : app,
    status: failure ? { errorCode: failure[0], failureReason: failure[1] } : { errorCode: 0, failureReason: 'Other.' },
    deviceDetail: { deviceId: '', operatingSystem: os, browser, isCompliant: r() < 0.6, isManaged: r() < 0.6 },
    location: { city, state, countryOrRegion: country, geoCoordinates: { latitude: lat, longitude: lon } },
  };
  const attributes: Record<string, JsonValue> = {
    ...base('SignInLogs', 'Sign-in activity', at, ip, correlation),
    resultType: String(failure?.[0] ?? 0),
    identity: person[1],
    location: country,
    properties,
    evt: { category: 'SignInLogs', name: 'Sign-in activity', outcome: failure ? 'failure' : 'success' },
    usr: { id: person[0], email: person[0], name: person[1] },
    network: { client: { ip } },
    title: `'Sign-in activity' resulted in ${failure ? 'failure' : 'success'} initiated by ${person[0]} `,
  };
  if (failure) attributes.resultDescription = failure[1];
  return {
    id: `entra-signin-${at}-${i}`,
    timestamp: at,
    status: 'info',
    service: 'azure',
    host: '',
    source: 'azure.activedirectory',
    message: failure ? failure[1] : `${person[0]} signed in to ${app}`,
    tags: {},
    attributes,
  };
}

/** [activity, category, operation type, target type]. */
const ACTIVITIES: [string, string, string, string][] = [
  ['Update user', 'UserManagement', 'Update', 'User'],
  ['Add user', 'UserManagement', 'Add', 'User'],
  ['Reset password (by admin)', 'UserManagement', 'Update', 'User'],
  ['Add member to group', 'GroupManagement', 'Assign', 'User'],
  ['Remove member from group', 'GroupManagement', 'Unassign', 'User'],
  ['Add member to role', 'RoleManagement', 'Assign', 'User'],
  ['Remove member from role', 'RoleManagement', 'Unassign', 'User'],
  ['Consent to application', 'ApplicationManagement', 'Assign', 'ServicePrincipal'],
  ['Update application', 'ApplicationManagement', 'Update', 'Application'],
  ['Update conditional access policy', 'Policy', 'Update', 'Policy'],
];
const ROLES = ['Global Administrator', 'Security Reader', 'Reports Reader', 'User Administrator', 'Application Administrator'];
const GROUPS = ['storefront-engineers', 'billing-oncall', 'catalog-admins', 'all-employees'];

function audit(r: Rand, i: number, at: number): LogEvent {
  const [activity, category, operationType, targetType] = r() < 0.35 ? ACTIVITIES[0] : pick(r, ACTIVITIES);
  const admin = pick(r, ADMINS);
  const target = pick(r, PEOPLE.slice(2));
  const ip = PLACES[admin[3]][4];
  const correlation = uuid(r);
  const role = category === 'RoleManagement' ? (r() < 0.25 ? ROLES[0] : pick(r, ROLES.slice(1))) : null;
  const modified: JsonValue[] = role
    ? [
        { displayName: 'Role.ObjectID', newValue: `"${uuid(r)}"` },
        { displayName: 'Role.DisplayName', newValue: `"${role}"` },
      ]
    : category === 'GroupManagement'
      ? [{ displayName: 'Group.DisplayName', newValue: `"${pick(r, GROUPS)}"` }]
      : [];
  const named = targetType === 'User' ? target[0] : targetType === 'Policy' ? 'Require MFA for admins' : pick(r, APPS);
  const failed = r() < 0.03;
  const properties: Record<string, JsonValue> = {
    id: `Directory_${correlation}_${i}`,
    category,
    correlationId: correlation,
    result: failed ? 'failure' : 'success',
    resultReason: '',
    activityDisplayName: activity,
    activityDateTime: iso(at),
    loggedByService: category === 'Policy' ? 'Conditional Access' : 'Core Directory',
    operationType,
    initiatedBy: { user: { id: admin[2], userPrincipalName: admin[0], ipAddress: ip } },
    targetResources: [
      targetType === 'User'
        ? { id: target[2], type: 'User', userPrincipalName: target[0], modifiedProperties: modified }
        : { id: uuid(r), type: targetType, displayName: named, modifiedProperties: modified },
    ],
    additionalDetails: [],
  };
  const outcome = failed ? 'failure' : 'success';
  return {
    id: `entra-audit-${at}-${i}`,
    timestamp: at,
    status: 'info',
    service: 'azure',
    host: '',
    source: 'azure.activedirectory',
    message: `${activity}: ${named} (by ${admin[0]})`,
    tags: {},
    attributes: {
      ...base('AuditLogs', activity, at, ip, correlation),
      identity: admin[0],
      properties,
      evt: { category: 'AuditLogs', name: activity, outcome },
      usr: { id: admin[0], name: admin[0] },
      network: { client: { ip } },
      title: `'${activity}' resulted in ${outcome} initiated by ${admin[0]} `,
    },
  };
}

const RISK_TYPES = ['unfamiliarFeatures', 'anonymizedIPAddress', 'maliciousIPAddress', 'leakedCredentials', 'passwordSpray'];

function riskDetection(r: Rand, i: number, at: number): LogEvent {
  const person = pick(r, PEOPLE);
  const [city, state, lat, lon, ip] = PLACES.NG;
  const kind = pick(r, RISK_TYPES);
  const level = kind === 'leakedCredentials' || kind === 'maliciousIPAddress' ? 'high' : r() < 0.5 ? 'medium' : 'low';
  const correlation = uuid(r);
  return {
    id: `entra-risk-${at}-${i}`,
    timestamp: at,
    status: 'info',
    service: 'azure',
    host: '',
    source: 'azure.activedirectory',
    message: `Risk detected for ${person[0]}: ${kind} (${level} risk)`,
    tags: {},
    attributes: {
      ...base('UserRiskEvents', 'User Risk Detection', at, ip, correlation),
      identity: person[1],
      properties: {
        id: uuid(r),
        riskEventType: kind,
        riskState: 'atRisk',
        riskLevel: level,
        riskDetail: 'none',
        source: 'IdentityProtection',
        detectionTimingType: kind === 'leakedCredentials' ? 'offline' : 'realtime',
        activity: kind === 'leakedCredentials' ? 'user' : 'signin',
        ipAddress: ip,
        activityDateTime: iso(at),
        detectedDateTime: iso(at),
        userId: person[2],
        userDisplayName: person[1],
        userPrincipalName: person[0],
        location: { city, state, countryOrRegion: 'NG', geoCoordinates: { latitude: lat, longitude: lon } },
      },
      evt: { category: 'UserRiskEvents', name: 'User Risk Detection' },
      network: { client: { ip } },
    },
  };
}

/** A week of the tenant's sign-ins (denser in the last hours), directory changes and risk detections. */
export function entraLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  const spread = (n: number, span: number, make: (r: Rand, i: number, at: number) => LogEvent) => {
    for (let i = 0; i < n; i++) out.push(make(r, out.length, Math.round(now - r() * span)));
  };
  spread(500, HOUR, signIn);
  spread(900, 24 * HOUR, signIn);
  spread(900, 7 * 24 * HOUR, signIn);
  spread(15, HOUR, audit);
  spread(40, 24 * HOUR, audit);
  spread(160, 7 * 24 * HOUR, audit);
  spread(4, HOUR, riskDetection);
  spread(10, 24 * HOUR, riskDetection);
  spread(30, 7 * 24 * HOUR, riskDetection);
  return out;
}
