// The FleetDM dashboard. Datadog has no Fleet integration, so this is ObserveCraft's own, over what the FleetDM
// integration collects (server/crates/oc-api/src/fleetdm): the hosts' `fleetdm.host.*` gauges, the host counts
// `fleetdm.hosts.*`, the global policies' `fleetdm.policy.*`, the vulnerabilities' `fleetdm.vulnerability.*` and the
// activities, logs with `source:fleetdm` (`@evt.name` the activity type, `@usr.*` the actor,
// `@fleetdm.host_display_name` the host acted on, `@fleetdm.category` from the FleetDM log pipeline, which saving the
// connection installs). Written in the
// Datadog widget format the integration dashboards use and converted like them (import-integration-dashboards.mjs).
// node scripts/fleetdm-dashboard.mjs   (writes server/seeds/integrations/fleetdm/{dashboards.json,units.csv})
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import {
  BAD,
  GOOD,
  NONE,
  WARN,
  anyOf,
  bars,
  buildOwnDashboard,
  geomap,
  layout,
  logStream,
  logs,
  metric,
  note,
  pie,
  queryValue,
  table,
  timeseries,
  toplist,
  treemap,
} from './own-dashboard-widgets.mjs';

const HOST = 'fleetdm.host';
const COUNTS = 'fleetdm.hosts';
const POLICY = 'fleetdm.policy';
const VULN = 'fleetdm.vulnerability';
const ACTIVITIES = 'source:fleetdm';
const BY_HOST = '{fleet_host,platform,os_version}';

/** Activity types, as Fleet names them. */
const LOGINS = ['user_logged_in', 'user_failed_login', 'user_added_by_sso'];
const ACTIONS = [
  'ran_script',
  'installed_software',
  'uninstalled_software',
  'locked_host',
  'unlocked_host',
  'wiped_host',
  'read_host_disk_encryption_key',
  'mdm_enrolled',
  'mdm_unenrolled',
  'transferred_hosts',
];
const CHANGES = [
  'created_policy',
  'edited_policy',
  'deleted_policy',
  'applied_spec_policy',
  'created_saved_query',
  'edited_saved_query',
  'deleted_saved_query',
  'applied_spec_saved_query',
  'live_query',
  'edited_agent_options',
  'added_script',
  'deleted_script',
  'added_software',
  'deleted_software',
];

function dashboard() {
  const { at, group } = layout();
  const intro = group(
    '',
    'vivid_blue',
    [
      at(
        note(
          "**Fleet** (FleetDM) manages and queries laptops, desktops and servers with osquery, and their MDM. This dashboard shows the hosts of the connected Fleet server (online or not, last seen, platforms and OS versions, disk space, failing policies, vulnerabilities, MDM enrollment and disk encryption), its global policies and its activity feed: logins, policies and queries changed, live queries, scripts run, software installed, hosts locked or wiped.\n\nConnect a Fleet server in [Integrations → FleetDM](/integrations/fleetdm): ObserveCraft reads its API every minute, for self-hosted Fleet and Fleet's cloud alike.",
          'white',
        ),
        0,
        0,
        12,
        2,
      ),
    ],
    0,
    5,
    { show_title: false, banner_img: '/images/integrations/fleetdm-banner.svg' },
  );
  const overview = group(
    'Overview',
    'blue',
    [
      at(queryValue('Hosts', [metric(`sum:${COUNTS}.total{*}`)], { formats: NONE }), 0, 0, 2, 2),
      at(queryValue('Online', [metric(`sum:${COUNTS}.online{*}`)], { formats: GOOD }), 2, 0, 2, 2),
      at(queryValue('Offline', [metric(`sum:${COUNTS}.offline{*}`)], { formats: WARN }), 4, 0, 2, 2),
      at(queryValue('Missing', [metric(`sum:${COUNTS}.mia{*}`)], { formats: BAD }), 6, 0, 2, 2),
      at(queryValue('New today', [metric(`sum:${COUNTS}.new{*}`)], { formats: NONE }), 8, 0, 2, 2),
      at(queryValue('Low disk space', [metric(`sum:${COUNTS}.low_disk_space{*}`)], { formats: WARN }), 10, 0, 2, 2),
      at(queryValue('Failing checks', [metric(`sum:${HOST}.failing_policies{*}`)], { formats: BAD }), 0, 2, 2, 2),
      at(queryValue('Vulnerabilities', [metric('sum:fleetdm.vulnerabilities.total{*}')], { formats: WARN }), 2, 2, 2, 2),
      at(queryValue('Exploited CVEs', [metric(`sum:${VULN}.hosts{known_exploited:true} by {cve}`)], { formula: 'count_nonzero(query1)', formats: BAD }), 4, 2, 2, 2),
      at(
        queryValue('MDM enrolled', [metric(`sum:${HOST}.mdm_enrolled{*}`), metric(`sum:${HOST}.count{*}`, 'last', 'query2')], { formula: '100 * query1 / query2', formats: NONE, unit: '%' }),
        6,
        2,
        2,
        2,
      ),
      at(
        queryValue('Encrypted', [metric(`sum:${HOST}.disk_encrypted{*}`), metric(`sum:${HOST}.count{*}`, 'last', 'query2')], { formula: '100 * query1 / query2', formats: NONE, unit: '%' }),
        8,
        2,
        2,
        2,
      ),
      at(queryValue('Activities', [logs(ACTIVITIES)], { formats: NONE, timeseries: true }), 10, 2, 2, 2),
    ],
    5,
    5,
  );
  const hosts = group(
    'Hosts',
    'green',
    [
      at(geomap('Hosts around the world', metric(`sum:${HOST}.count{*} by {country_code}`)), 0, 0, 8, 5),
      at(
        pie(
          'Status',
          [metric(`sum:${COUNTS}.online{*}`, 'last', 'query1'), metric(`sum:${COUNTS}.offline{*}`, 'last', 'query2'), metric(`sum:${COUNTS}.mia{*}`, 'last', 'query3')],
          [
            { alias: 'online', formula: 'query1' },
            { alias: 'offline', formula: 'query2' },
            { alias: 'missing', formula: 'query3' },
          ],
        ),
        8,
        0,
        4,
        5,
      ),
      at(pie('Platforms', [metric(`sum:${COUNTS}.by_platform{*} by {platform}`)]), 0, 5, 4, 4),
      at(pie('Teams', [metric(`sum:${HOST}.count{*} by {team}`)]), 4, 5, 4, 4),
      at(bars('osquery versions', metric(`sum:${HOST}.count{*} by {osquery_version}`)), 8, 5, 4, 4),
      at(
        timeseries('Hosts by status', [metric(`sum:${COUNTS}.online{*}`, 'avg', 'query1'), metric(`sum:${COUNTS}.offline{*}`, 'avg', 'query2'), metric(`sum:${COUNTS}.mia{*}`, 'avg', 'query3')], {
          formulas: [
            { alias: 'online', formula: 'query1' },
            { alias: 'offline', formula: 'query2' },
            { alias: 'missing', formula: 'query3' },
          ],
          display: 'area',
        }),
        0,
        9,
        6,
        3,
      ),
      at(treemap('OS versions', metric(`sum:${HOST}.count{*} by {os_version}`)), 6, 9, 6, 3),
      at(timeseries('Online hosts by platform', [metric(`sum:${HOST}.online{*} by {platform}`, 'avg')], { display: 'area' }), 0, 12, 4, 3),
      at(toplist('Longest unseen hosts', metric(`max:${HOST}.seen_age{*} by {fleet_host}`)), 4, 12, 4, 3),
      at(toplist('Least free disk space', metric(`min:${HOST}.disk.available_percent{*} by {fleet_host}`), { order: 'asc' }), 8, 12, 4, 3),
      at(
        table('Hosts', [
          ['failing policies', `max:${HOST}.failing_policies{*} by ${BY_HOST}`],
          ['online', `max:${HOST}.online{*} by ${BY_HOST}`],
          ['seconds since seen', `max:${HOST}.seen_age{*} by ${BY_HOST}`],
          ['critical vulnerabilities', `max:${HOST}.critical_vulnerabilities{*} by ${BY_HOST}`],
          ['free disk %', `min:${HOST}.disk.available_percent{*} by ${BY_HOST}`],
          ['uptime', `max:${HOST}.uptime{*} by ${BY_HOST}`],
        ]),
        0,
        15,
        12,
        4,
      ),
    ],
    10,
    19,
  );
  const compliance = group(
    'Policies and security',
    'orange',
    [
      at(queryValue('Hosts failing a policy', [metric(`sum:${HOST}.failing_policies{*} by {fleet_host_id}`)], { formula: 'count_nonzero(query1)', formats: BAD }), 0, 0, 3, 2),
      at(queryValue('Critical vulnerabilities', [metric(`sum:${HOST}.critical_vulnerabilities{*}`)], { formats: BAD }), 3, 0, 3, 2),
      at(queryValue('MDM enrolled', [metric(`sum:${HOST}.mdm_enrolled{*}`)], { formats: GOOD }), 6, 0, 3, 2),
      at(queryValue('Disks encrypted', [metric(`sum:${HOST}.disk_encrypted{*}`)], { formats: GOOD }), 9, 0, 3, 2),
      at(
        pie(
          'Policy checks',
          [metric(`sum:${POLICY}.passing_hosts{*}`, 'last', 'query1'), metric(`sum:${POLICY}.failing_hosts{*}`, 'last', 'query2')],
          [
            { alias: 'passing', formula: 'query1' },
            { alias: 'failing', formula: 'query2' },
          ],
        ),
        0,
        2,
        4,
        4,
      ),
      at(
        table('Global policies', [
          ['failing hosts', `sum:${POLICY}.failing_hosts{*} by {policy,critical}`],
          ['passing hosts', `sum:${POLICY}.passing_hosts{*} by {policy,critical}`],
        ]),
        4,
        2,
        8,
        4,
      ),
      at(bars('Most failing policies', metric(`sum:${POLICY}.failing_hosts{*} by {policy}`)), 0, 6, 4, 3),
      at(toplist('Hosts with the most failing policies', metric(`max:${HOST}.failing_policies{*} by {fleet_host}`)), 4, 6, 4, 3),
      at(toplist('Most critical vulnerabilities', metric(`max:${HOST}.critical_vulnerabilities{*} by {fleet_host}`)), 8, 6, 4, 3),
    ],
    29,
    10,
  );
  const vulnerabilities = group(
    'Vulnerabilities',
    'red',
    [
      at(toplist('Vulnerabilities on the most hosts', metric(`max:${VULN}.hosts{*} by {cve}`), { palette: 'warm' }), 0, 0, 4, 4),
      at(pie('By severity', [metric(`sum:${VULN}.hosts{*} by {severity}`)]), 4, 0, 4, 4),
      at(pie('Known exploited (CISA)', [metric(`sum:${VULN}.hosts{*} by {known_exploited}`)]), 8, 0, 4, 4),
      at(table('Vulnerabilities', [['hosts', `max:${VULN}.hosts{*} by {cve,severity,known_exploited}`]]), 0, 4, 6, 4),
      at(
        timeseries('Vulnerabilities found', [metric('sum:fleetdm.vulnerabilities.total{*}', 'avg')], { formulas: [{ alias: 'vulnerabilities', formula: 'query1' }], display: 'area', palette: 'warm' }),
        6,
        4,
        6,
        4,
      ),
    ],
    39,
    8,
  );
  const mdm = group(
    'MDM and disk encryption',
    'green',
    [
      at(pie('MDM enrollment', [metric(`sum:${HOST}.count{*} by {mdm_status}`)]), 0, 0, 4, 4),
      at(
        pie(
          'Disk encryption',
          [metric(`sum:${HOST}.disk_encrypted{*}`, 'last', 'query1'), metric(`sum:${HOST}.count{*}`, 'last', 'query2')],
          [
            { alias: 'encrypted', formula: 'query1' },
            { alias: 'not encrypted or unknown', formula: 'query2 - query1' },
          ],
        ),
        4,
        0,
        4,
        4,
      ),
      at(bars('MDM enrolled by platform', metric(`sum:${HOST}.mdm_enrolled{*} by {platform}`)), 8, 0, 4, 4),
      at(logStream('MDM enrollment, profiles and disk encryption', `${ACTIVITIES} @fleetdm.category:mdm`), 0, 4, 12, 4),
    ],
    47,
    8,
  );
  const activity = group(
    'Activity feed',
    'purple',
    [
      at(timeseries('Activities over time', [logs(ACTIVITIES, { groupBy: ['@evt.name'] })], { display: 'bars' }), 0, 0, 8, 3),
      at(queryValue('Failed logins', [logs(`${ACTIVITIES} @evt.name:user_failed_login`)], { formats: BAD, timeseries: true }), 8, 0, 4, 3),
      at(pie('Activities by category', [logs(ACTIVITIES, { groupBy: ['@fleetdm.category'] })]), 0, 3, 4, 3),
      at(toplist('Most active users', logs(`${ACTIVITIES} @usr.email:*`, { groupBy: ['@usr.email'] }), { palette: 'purple' }), 4, 3, 4, 3),
      at(toplist('Hosts acted on most', logs(`${ACTIVITIES} @fleetdm.host_display_name:*`, { groupBy: ['@fleetdm.host_display_name'] }), { palette: 'cool' }), 8, 3, 4, 3),
      at(logStream('Logins and failed logins', `${ACTIVITIES} ${anyOf(LOGINS)}`), 0, 6, 6, 4),
      at(logStream('Scripts, software and MDM actions on hosts', `${ACTIVITIES} ${anyOf(ACTIONS)}`), 6, 6, 6, 4),
      at(logStream('Policies, queries and configuration changed', `${ACTIVITIES} ${anyOf(CHANGES)}`), 0, 10, 6, 4),
      at(logStream('Warnings and errors', `${ACTIVITIES} status:(warn OR error)`), 6, 10, 6, 4),
      at(timeseries('Activities by user', [logs(`${ACTIVITIES} @usr.email:*`, { groupBy: ['@usr.email'] })], { display: 'bars', palette: 'purple' }), 0, 14, 6, 3),
      at(bars('Top activity types', logs(ACTIVITIES, { groupBy: ['@evt.name'] })), 6, 14, 6, 3),
    ],
    55,
    17,
  );
  return {
    title: 'FleetDM Overview',
    description: 'Hosts of a Fleet server (status, platforms, OS versions, disk, failing policies, MDM, disk encryption), its global policies, its vulnerabilities and its activity feed.',
    layout_type: 'ordered',
    template_variables: [],
    widgets: [intro, overview, hosts, compliance, vulnerabilities, mdm, activity],
  };
}

/** Units of the metrics the dashboard charts (Datadog unit names): the poller sends none. */
export const FLEETDM_UNITS = {
  'fleetdm.host.seen_age': 'second',
  'fleetdm.host.uptime': 'second',
  'fleetdm.host.memory': 'byte',
  'fleetdm.host.disk.available': 'byte',
  'fleetdm.host.disk.total': 'byte',
  'fleetdm.host.disk.available_percent': 'percent',
};

export const buildDashboard = () => buildOwnDashboard(dashboard(), { integration: 'fleetdm', name: 'FleetDM', script: 'scripts/fleetdm-dashboard.mjs' });

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dir = new URL('../server/seeds/integrations/fleetdm/', import.meta.url);
  mkdirSync(dir, { recursive: true });
  writeFileSync(new URL('dashboards.json', dir), JSON.stringify([buildDashboard()], null, 2) + '\n');
  writeFileSync(
    new URL('units.csv', dir),
    `# Units of the FleetDM integration's metrics (scripts/fleetdm-dashboard.mjs; the poller sends none).\n${Object.entries(FLEETDM_UNITS)
      .map(([name, unit]) => `${name},${unit}`)
      .join('\n')}\n`,
  );
  console.log('wrote server/seeds/integrations/fleetdm/{dashboards.json,units.csv}');
}
