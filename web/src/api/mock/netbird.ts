// The mock NetBird audit log (`source:netbird`), shaped as the NetBird connector package maps it
// (integrations/connectors/netbird/connector.yaml): its dashboard and Logs → source:netbird in mock mode. The connection
// itself is the generic connectors mock (mock/connectors.ts).
import type { JsonValue, LogEvent, LogStatus } from '../types';
import type { Rand } from './generator';

const HOUR = 3_600_000;
const HOST = 'netbird.example.com';

// ---------------------------------------------------------------- the audit log

const PEOPLE = [
  ['google-oauth2|1001', 'Jordan Patel', 'jordan.patel@example.com'],
  ['google-oauth2|1002', 'Sam Lee', 'sam.lee@example.com'],
  ['google-oauth2|1003', 'Alex Morgan', 'alex.morgan@example.com'],
] as const;
const PEERS = ['laptop-jordan', 'laptop-sam', 'laptop-alex', 'build-runner-1', 'build-runner-2', 'i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60003', 'routing-peer-eu'];

/** activity code, activity, weight, what it touched (none: a dashboard login) */
const ACTIVITIES: [string, string, number, ((r: Rand) => string) | null][] = [
  ['dashboard.login', 'Dashboard login', 30, null],
  ['user.peer.login', 'User logged in peer', 24, (r) => PEERS[Math.floor(r() * 3)]],
  ['peer.setupkey.add', 'Peer added', 6, (r) => `build-runner-${1 + Math.floor(r() * 2)}`],
  ['peer.user.add', 'Peer added', 5, (r) => PEERS[Math.floor(r() * 3)]],
  ['user.peer.delete', 'Peer deleted', 3, (r) => PEERS[3 + Math.floor(r() * 2)]],
  ['peer.login.expire', 'Peer login expired', 4, (r) => PEERS[Math.floor(r() * PEERS.length)]],
  ['policy.update', 'Policy updated', 5, (r) => ['storefront-api access', 'ci runners to staging', 'admins to all'][Math.floor(r() * 3)]],
  ['policy.add', 'Policy added', 2, () => 'billing database access'],
  ['route.update', 'Route updated', 3, () => 'vpc-eu-west-1'],
  ['group.update', 'Group updated', 4, (r) => ['developers', 'ci-runners', 'servers'][Math.floor(r() * 3)]],
  ['setupkey.add', 'Setup key created', 2, () => 'ci-runners'],
  ['setupkey.revoke', 'Setup key revoked', 1, () => 'onboarding-2026-10'],
  ['setupkey.overuse', 'Setup key overused', 1, () => 'ci-runners'],
  ['user.invite', 'User invited', 2, () => 'riley.chen@example.com'],
  ['user.block', 'User blocked', 1, () => 'former.contractor@example.com'],
  ['nameserver.group.update', 'Nameserver group updated', 1, () => 'internal-dns'],
  ['network.resource.create', 'Network resource created', 2, () => 'orders-db.internal'],
  ['personal.access.token.create', 'Personal access token created', 1, () => 'observecraft-reader'],
];
const TOTAL = ACTIVITIES.reduce((n, a) => n + a[2], 0);
const ERRORS = ['setupkey.overuse'];
const WARNINGS = ['user.block', 'peer.login.expire', 'setupkey.revoke'];
/** Events without a person behind them. */
const SYSTEM = ['peer.login.expire', 'setupkey.overuse'];

/** A week of audit events, as the NetBird poller stores them. */
export function netbirdLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (let i = 0; i < 320; i++) {
    const ts = Math.round(now - r() * 7 * 24 * HOUR);
    let pick = r() * TOTAL;
    const [code, activity, , target] = ACTIVITIES.find((a) => (pick -= a[2]) < 0) ?? ACTIVITIES[0];
    const subject = target?.(r);
    const person = SYSTEM.includes(code) ? null : PEOPLE[Math.floor(r() * PEOPLE.length)];
    const status: LogStatus = ERRORS.includes(code) ? 'error' : WARNINGS.includes(code) ? 'warn' : 'info';
    const netbird: Record<string, JsonValue> = { event_id: String(5000 + i), category: code.split('.')[0], activity };
    if (subject) Object.assign(netbird, { target: subject, meta: { name: subject } });
    out.push({
      id: `netbird-${ts}-${i}`,
      timestamp: ts,
      status,
      service: 'netbird',
      host: HOST,
      source: 'netbird',
      message: `${activity}${subject ? `: ${subject}` : ''}${person ? ` (by ${person[1]})` : ''}`,
      tags: {},
      attributes: {
        evt: { name: code, outcome: status === 'error' ? 'failure' : 'success' },
        ...(person ? { usr: { id: person[0], name: person[1], email: person[2] } } : {}),
        netbird,
      },
    });
  }
  return out;
}
