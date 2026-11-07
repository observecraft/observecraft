// Mock `netbird.peer.*` metrics of the NetBird integration (server/crates/oc-api/src/netbird/peers.rs): one series
// per peer with the tags the poller puts on them. Fictional peers: the demo's hosts, CI runners and laptops.
import type { CloudMetricDef } from './cloudMetrics';

type Tags = Record<string, string>;

const INSTANCE = 'netbird.example.com';
/** name, OS, client version, country, city, connected, login expired, hours since seen when disconnected */
const PEERS: [string, string, string, string, string, boolean, boolean, number][] = [
  ['i-0a1b2c3d4e5f60001', 'Amazon Linux 2023', '0.59.1', 'US', 'Ashburn', true, false, 0],
  ['i-0a1b2c3d4e5f60002', 'Amazon Linux 2023', '0.59.1', 'US', 'Ashburn', true, false, 0],
  ['i-0a1b2c3d4e5f60003', 'Ubuntu 24.04', '0.59.1', 'IE', 'Dublin', true, false, 0],
  ['i-0a1b2c3d4e5f60004', 'Ubuntu 24.04', '0.58.0', 'IE', 'Dublin', true, false, 0],
  ['build-runner-1', 'Ubuntu 24.04', '0.58.0', 'DE', 'Frankfurt', false, true, 30],
  ['build-runner-2', 'Ubuntu 24.04', '0.59.1', 'DE', 'Frankfurt', true, false, 0],
  ['laptop-jordan', 'Darwin 15.1', '0.59.1', 'RO', 'Bucharest', true, false, 0],
  ['laptop-sam', 'Darwin 15.1', '0.59.1', 'GB', 'London', false, false, 3],
  ['laptop-alex', 'Windows 11 Pro', '0.57.2', 'NL', 'Amsterdam', true, false, 0],
  ['laptop-riley', 'Windows 11 Pro', '0.59.1', 'US', 'Austin', false, false, 14],
  ['phone-jordan', 'iOS 18.1', '0.59.0', 'RO', 'Bucharest', false, false, 1],
  ['routing-peer-eu', 'Debian 12', '0.59.1', 'IE', 'Dublin', true, false, 0],
];

const STATE = new Map(PEERS.map(([name, , , , , connected, expired, hours]) => [name, { connected, expired, hours }]));

export const NETBIRD_SETS: Record<string, Tags[]> = {
  netbird_peers: PEERS.map(([peer, os, version, country, city], i) => ({
    netbird_instance: INSTANCE,
    peer,
    peer_id: `cs1tnh0hhcjnqoiuebf${i.toString(36)}`,
    os,
    netbird_version: version,
    country_code: country,
    city,
  })),
};

const peer = (e: Tags) => STATE.get(e.peer) ?? { connected: false, expired: false, hours: 0 };
const flag = (description: string, base: (e: Tags) => boolean): CloudMetricDef => ({ set: 'netbird_peers', description, base: (e) => (base(e) ? 1 : 0), noise: 0, integer: true });

export const NETBIRD_METRICS: Record<string, CloudMetricDef> = {
  'netbird.peer.count': flag('NetBird peers', () => true),
  'netbird.peer.connected': flag('NetBird peers connected', (e) => peer(e).connected),
  'netbird.peer.login_expired': flag('NetBird peers whose login expired', (e) => peer(e).expired),
  'netbird.peer.approval_required': flag('NetBird peers waiting for approval', (e) => e.peer === 'laptop-riley'),
  'netbird.peer.ssh_enabled': flag('NetBird peers with the SSH server enabled', (e) => e.peer.startsWith('i-') || e.peer.startsWith('routing')),
  'netbird.peer.last_seen_age': { set: 'netbird_peers', description: 'Seconds since the NetBird peer was last seen', unit: 'second', base: (e) => peer(e).hours * 3600, noise: 0.02, integer: true },
  'netbird.peer.accessible_peers': { set: 'netbird_peers', description: 'Peers the NetBird peer can reach', base: (e) => (e.peer.startsWith('laptop') ? 6 : 3), noise: 0, integer: true },
};
