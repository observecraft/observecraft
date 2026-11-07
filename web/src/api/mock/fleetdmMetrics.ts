// Mock `fleetdm.*` metrics of the FleetDM integration (server/crates/oc-api/src/fleetdm/hosts.rs): one series per
// host with the tags the poller puts on them, Fleet's host counts and its global policies. Fictional hosts: the
// demo's EC2 instances, CI runners, laptops and desktops.
import type { CloudMetricDef } from './cloudMetrics';

type Tags = Record<string, string>;

const INSTANCE = 'fleet.example.com';
const GB = 1e9;

interface Host {
  name: string;
  platform: string;
  os: string;
  osquery: string;
  team: string;
  model?: string;
  status: 'online' | 'offline' | 'missing';
  /** hours since seen when not online */
  hours: number;
  diskGb: number;
  freePct: number;
  memoryGb: number;
  failing: number;
  critical: number;
  mdm?: boolean;
  encrypted?: boolean;
}

const h = (
  name: string,
  platform: string,
  os: string,
  osquery: string,
  team: string,
  status: Host['status'],
  hours: number,
  diskGb: number,
  freePct: number,
  memoryGb: number,
  failing: number,
  critical: number,
  extra: Partial<Host> = {},
): Host => ({
  name,
  platform,
  os,
  osquery,
  team,
  status,
  hours,
  diskGb,
  freePct,
  memoryGb,
  failing,
  critical,
  ...extra,
});

const MAC = { model: 'MacBookPro18,3', mdm: true, encrypted: true };
const WIN = { model: 'Latitude 7440', mdm: true };

const HOSTS: Host[] = [
  h("Jordan's MacBook Pro", 'darwin', 'macOS 15.1', '5.14.1', 'Workstations', 'online', 0, 494, 24, 32, 1, 2, MAC),
  h("Sam's MacBook Pro", 'darwin', 'macOS 15.1', '5.14.1', 'Workstations', 'online', 0, 494, 61, 16, 0, 0, MAC),
  h("Alex's MacBook Air", 'darwin', 'macOS 14.7', '5.13.1', 'Workstations', 'offline', 5, 245, 9, 16, 2, 4, { ...MAC, model: 'Mac14,2', encrypted: false }),
  h("Riley's MacBook Pro", 'darwin', 'macOS 15.1', '5.14.1', 'Workstations', 'online', 0, 994, 70, 36, 0, 0, MAC),
  h('desktop-sam', 'windows', 'Windows 11 Pro 23H2', '5.13.0', 'Office', 'missing', 24 * 36, 512, 33, 32, 2, 1, { ...WIN, mdm: false, encrypted: false }),
  h('laptop-casey', 'windows', 'Windows 11 Pro 24H2', '5.14.1', 'Workstations', 'online', 0, 512, 45, 16, 0, 0, { ...WIN, encrypted: true }),
  h('laptop-morgan', 'windows', 'Windows 11 Pro 24H2', '5.14.1', 'Workstations', 'online', 0, 256, 12, 16, 1, 0, { ...WIN, encrypted: true }),
  h('i-0a1b2c3d4e5f60001', 'amzn', 'Amazon Linux 2023', '5.14.1', 'Servers', 'online', 0, 100, 58, 8, 0, 0),
  h('i-0a1b2c3d4e5f60002', 'amzn', 'Amazon Linux 2023', '5.14.1', 'Servers', 'online', 0, 100, 41, 8, 0, 1),
  h('i-0a1b2c3d4e5f60003', 'ubuntu', 'Ubuntu 24.04.1 LTS', '5.14.1', 'Servers', 'online', 0, 200, 72, 16, 0, 0),
  h('i-0a1b2c3d4e5f60004', 'ubuntu', 'Ubuntu 22.04.5 LTS', '5.12.2', 'Servers', 'offline', 2, 200, 18, 16, 1, 3),
  h('build-runner-1', 'ubuntu', 'Ubuntu 24.04.1 LTS', '5.14.1', 'CI', 'online', 0, 160, 7, 32, 1, 0),
];

/** Where Fleet located each host (country ISO code, city): the dashboard's world map. */
const GEO: Record<string, [string, string]> = {
  "Jordan's MacBook Pro": ['RO', 'Bucharest'],
  "Sam's MacBook Pro": ['GB', 'London'],
  "Alex's MacBook Air": ['NL', 'Amsterdam'],
  "Riley's MacBook Pro": ['US', 'Austin'],
  'desktop-sam': ['GB', 'London'],
  'laptop-casey': ['DE', 'Berlin'],
  'laptop-morgan': ['US', 'New York'],
  'i-0a1b2c3d4e5f60001': ['US', 'Ashburn'],
  'i-0a1b2c3d4e5f60002': ['US', 'Ashburn'],
  'i-0a1b2c3d4e5f60003': ['IE', 'Dublin'],
  'i-0a1b2c3d4e5f60004': ['IE', 'Dublin'],
  'build-runner-1': ['DE', 'Frankfurt'],
};

const BY_NAME = new Map(HOSTS.map((x) => [x.name, x]));
const host = (e: Tags) => BY_NAME.get(e.fleet_host) ?? HOSTS[0];

const POLICIES: [name: string, critical: boolean, platform: string][] = [
  ['Disk encryption enabled', true, 'darwin,windows'],
  ['Firewall enabled', false, ''],
  ['Screen lock after 15 minutes', false, 'darwin,windows'],
  ['osquery 5.13 or newer', false, ''],
  ['No critical vulnerabilities', true, ''],
];

/** Failing hosts per policy, from the hosts' failing counts. */
const FAILING: Record<string, number> = {
  'Disk encryption enabled': HOSTS.filter((x) => x.encrypted === false).length,
  'Firewall enabled': 1,
  'Screen lock after 15 minutes': 2,
  'osquery 5.13 or newer': HOSTS.filter((x) => x.osquery < '5.13').length,
  'No critical vulnerabilities': HOSTS.filter((x) => x.critical > 0).length,
};

const PLATFORMS = [...new Set(HOSTS.map((x) => x.platform))];

export const FLEETDM_SETS: Record<string, Tags[]> = {
  fleetdm_hosts: HOSTS.map((x, i) => ({
    fleet_instance: INSTANCE,
    fleet_host: x.name,
    fleet_host_id: String(i + 1),
    platform: x.platform,
    os_version: x.os,
    osquery_version: x.osquery,
    team: x.team,
    ...(x.model ? { hardware_model: x.model } : {}),
    ...(GEO[x.name] ? { country_code: GEO[x.name][0], city: GEO[x.name][1] } : {}),
  })),
  fleetdm_instance: [{ fleet_instance: INSTANCE }],
  fleetdm_platforms: PLATFORMS.map((platform) => ({ fleet_instance: INSTANCE, platform })),
  fleetdm_policies: POLICIES.map(([policy, critical, platform], i) => ({ fleet_instance: INSTANCE, policy, policy_id: String(12 + i), critical: String(critical), ...(platform ? { platform } : {}) })),
};

const flag = (description: string, base: (x: Host) => boolean): CloudMetricDef => ({ set: 'fleetdm_hosts', description, base: (e) => (base(host(e)) ? 1 : 0), noise: 0, integer: true });
const gauge = (description: string, base: (x: Host) => number, extra: Partial<CloudMetricDef> = {}): CloudMetricDef => ({
  set: 'fleetdm_hosts',
  description,
  base: (e) => base(host(e)),
  noise: 0,
  integer: true,
  ...extra,
});
const count = (description: string, n: number): CloudMetricDef => ({ set: 'fleetdm_instance', description, base: () => n, noise: 0, integer: true });
const status = (s: Host['status']) => HOSTS.filter((x) => x.status === s).length;

export const FLEETDM_METRICS: Record<string, CloudMetricDef> = {
  'fleetdm.host.count': flag('Fleet hosts', () => true),
  'fleetdm.host.online': flag('Fleet hosts online', (x) => x.status === 'online'),
  'fleetdm.host.offline': flag('Fleet hosts offline', (x) => x.status === 'offline'),
  'fleetdm.host.missing': flag('Fleet hosts missing', (x) => x.status === 'missing'),
  'fleetdm.host.seen_age': gauge('Seconds since the Fleet host last checked in', (x) => x.hours * 3600 + 15, { unit: 'second', noise: 0.02 }),
  'fleetdm.host.uptime': gauge('Fleet host uptime', (x) => (x.status === 'online' ? 4 * 86_400 : 0), { unit: 'second' }),
  'fleetdm.host.memory': gauge('Fleet host memory', (x) => x.memoryGb * 1024 ** 3, { unit: 'byte' }),
  'fleetdm.host.disk.total': gauge('Fleet host disk space', (x) => x.diskGb * GB, { unit: 'byte' }),
  'fleetdm.host.disk.available': gauge('Fleet host free disk space', (x) => (x.diskGb * x.freePct * GB) / 100, { unit: 'byte', noise: 0.01 }),
  'fleetdm.host.disk.available_percent': gauge('Fleet host free disk space', (x) => x.freePct, { unit: 'percent', integer: false, noise: 0.01 }),
  'fleetdm.host.failing_policies': gauge('Policies the Fleet host fails', (x) => x.failing),
  'fleetdm.host.critical_vulnerabilities': gauge('Critical vulnerabilities on the Fleet host', (x) => x.critical),
  'fleetdm.host.issues': gauge('Issues of the Fleet host', (x) => x.failing + x.critical),
  'fleetdm.host.mdm_enrolled': flag('Fleet hosts enrolled in MDM', (x) => x.mdm === true),
  'fleetdm.host.disk_encrypted': flag('Fleet hosts with an encrypted disk', (x) => x.encrypted === true),
  'fleetdm.hosts.total': count('Fleet hosts', HOSTS.length),
  'fleetdm.hosts.online': count('Fleet hosts online', status('online')),
  'fleetdm.hosts.offline': count('Fleet hosts offline', status('offline')),
  'fleetdm.hosts.mia': count('Fleet hosts missing', status('missing')),
  'fleetdm.hosts.missing_30_days': count('Fleet hosts missing for 30 days', status('missing')),
  'fleetdm.hosts.new': count('Fleet hosts enrolled in the last day', 0),
  'fleetdm.hosts.low_disk_space': count('Fleet hosts with little free disk space', HOSTS.filter((x) => x.freePct < 10).length),
  'fleetdm.hosts.by_platform': { set: 'fleetdm_platforms', description: 'Fleet hosts by platform', base: (e) => HOSTS.filter((x) => x.platform === e.platform).length, noise: 0, integer: true },
  'fleetdm.policy.failing_hosts': { set: 'fleetdm_policies', description: 'Hosts failing the Fleet policy', base: (e) => FAILING[e.policy] ?? 0, noise: 0, integer: true },
  'fleetdm.policy.passing_hosts': { set: 'fleetdm_policies', description: 'Hosts passing the Fleet policy', base: (e) => HOSTS.length - (FAILING[e.policy] ?? 0), noise: 0, integer: true },
};
