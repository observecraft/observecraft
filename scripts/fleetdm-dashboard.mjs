// The FleetDM dashboard. Datadog has no Fleet integration, so this is ObserveCraft's own, over what the FleetDM
// integration collects (server/crates/oc-api/src/fleetdm): the hosts' `fleetdm.host.*` gauges, the host counts
// `fleetdm.hosts.*`, the global policies' `fleetdm.policy.*` and the activities, logs with `source:fleetdm`
// (`@evt.name` the activity type, `@usr.*` the actor, `@fleetdm.host_display_name` the host acted on). Written in the
// Datadog widget format the integration dashboards use and converted like them (import-integration-dashboards.mjs).
// node scripts/fleetdm-dashboard.mjs   (writes server/seeds/integrations/fleetdm/{dashboards.json,units.csv})
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { convertDashboard } from './import-integration-dashboards.mjs';

// ---------------------------------------------------------------- queries (Datadog format)

const metric = (query, aggregator = 'last', name = 'query1') => ({ data_source: 'metrics', name, query, aggregator });

/** A log query: a count, grouped by facets. */
const logs = (search, { name = 'query1', groupBy = [], limit = 10 } = {}) => ({
  data_source: 'logs',
  name,
  search: { query: search },
  indexes: ['*'],
  compute: { aggregation: 'count' },
  group_by: groupBy.map((f) => ({ facet: f, limit, sort: { aggregation: 'count', order: 'desc' } })),
  storage: 'hot',
});

// ---------------------------------------------------------------- widgets (Datadog format)

const head = (title) => ({ title, title_size: '16', title_align: 'left' });

function queryValue(title, queries, { formula = 'query1', formats = [], timeseries = false, unit } = {}) {
  return {
    type: 'query_value',
    ...head(title),
    requests: [{ response_format: 'scalar', queries, formulas: [{ formula }], conditional_formats: formats }],
    autoscale: true,
    precision: 0,
    ...(unit ? { custom_unit: unit } : {}),
    ...(timeseries ? { timeseries_background: { type: 'bars' } } : {}),
  };
}

function timeseries(title, queries, { formulas, display = 'line', palette = 'dog_classic' } = {}) {
  return {
    type: 'timeseries',
    ...head(title),
    show_legend: true,
    legend_layout: 'auto',
    requests: [{ response_format: 'timeseries', queries, formulas: formulas ?? queries.map((q) => ({ formula: q.name })), style: { palette, order_by: 'values' }, display_type: display }],
    yaxis: { include_zero: true },
  };
}

function toplist(title, query, { count = 10, palette, order = 'desc' } = {}) {
  return {
    type: 'toplist',
    ...head(title),
    requests: [
      { response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }], sort: { count, order_by: [{ type: 'formula', index: 0, order }] }, ...(palette ? { style: { palette } } : {}) },
    ],
  };
}

/** Columns: [alias, query, aggregator]; rows: the first query's groups. */
function table(title, columns, count = 100) {
  const queries = columns.map(([, query, aggregator], i) => metric(query, aggregator ?? 'last', `query${i + 1}`));
  return {
    type: 'query_table',
    ...head(title),
    requests: [
      { response_format: 'scalar', queries, formulas: columns.map(([alias], i) => ({ alias, formula: `query${i + 1}` })), sort: { count, order_by: [{ type: 'formula', index: 0, order: 'desc' }] } },
    ],
  };
}

/** A world map of the query's groups by country (ISO code), in Datadog's `hostmap_blues` scale. */
const geomap = (title, query) => ({
  type: 'geomap',
  ...head(title),
  requests: [{ response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }], sort: { count: 250, order_by: [{ type: 'formula', index: 0, order: 'desc' }] } }],
  style: { palette: 'hostmap_blues', palette_flip: false },
  view: { focus: 'WORLD' },
});

/** A donut of the query's groups, one color each, with their share in the legend. */
const pie = (title, queries, formulas = [{ formula: 'query1' }]) => ({
  type: 'sunburst',
  ...head(title),
  requests: [{ response_format: 'scalar', queries, formulas, sort: { count: 12, order_by: [{ type: 'formula', index: 0, order: 'desc' }] }, style: { palette: 'dog_classic' } }],
  legend: { type: 'inline' },
  hide_total: false,
});

/** Rectangles sized by value, one color per group. */
const treemap = (title, query) => ({ type: 'treemap', ...head(title), requests: [{ response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }] }] });

/** Vertical bars, one color per group. */
const bars = (title, query, count = 10) => ({
  type: 'bar_chart',
  ...head(title),
  requests: [
    { response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }], sort: { count, order_by: [{ type: 'formula', index: 0, order: 'desc' }] }, style: { palette: 'dog_classic' } },
  ],
});

const logStream = (title, query) => ({
  type: 'log_stream',
  title,
  query,
  columns: ['host', 'service'],
  indexes: [],
  message_display: 'inline',
  show_date_column: true,
  show_message_column: true,
  sort: { column: 'time', order: 'desc' },
});

const note = (content, background_color = 'white', { align = 'left', size = '14' } = {}) => ({
  type: 'note',
  content,
  background_color,
  font_size: size,
  text_align: align,
  show_tick: false,
  tick_pos: '50%',
  tick_edge: 'left',
  has_padding: true,
});

const GOOD = [{ comparator: '>', value: 0, palette: 'green_on_white' }];
const BAD = [{ comparator: '>', value: 0, palette: 'red_on_white' }];
const WARN = [{ comparator: '>', value: 0, palette: 'yellow_on_white' }];
const NONE = [{ comparator: '>=', value: 0, palette: 'white_on_gray' }];

// ---------------------------------------------------------------- layout

let seq = 0;
const at = (definition, x, y, width, height) => ({ id: ++seq, definition, layout: { x, y, width, height } });
const group = (title, color, widgets, y, height, extra = {}) => at({ type: 'group', title, background_color: color, layout_type: 'ordered', widgets, ...extra }, 0, y, 12, height);

const HOST = 'fleetdm.host';
const COUNTS = 'fleetdm.hosts';
const POLICY = 'fleetdm.policy';
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
const anyOf = (types) => `@evt.name:(${types.join(' OR ')})`;

function dashboard() {
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
      at(queryValue('Failing policy checks', [metric(`sum:${HOST}.failing_policies{*}`)], { formats: BAD }), 8, 0, 2, 2),
      at(queryValue('Activities', [logs(ACTIVITIES)], { formats: NONE, timeseries: true }), 10, 0, 2, 2),
    ],
    5,
    3,
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
    8,
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
    27,
    10,
  );
  const activity = group(
    'Activity feed',
    'purple',
    [
      at(timeseries('Activities over time', [logs(ACTIVITIES, { groupBy: ['@evt.name'] })], { display: 'bars' }), 0, 0, 8, 3),
      at(queryValue('Failed logins', [logs(`${ACTIVITIES} @evt.name:user_failed_login`)], { formats: BAD, timeseries: true }), 8, 0, 4, 3),
      at(pie('Activities by type', [logs(ACTIVITIES, { groupBy: ['@evt.name'] })]), 0, 3, 4, 3),
      at(toplist('Most active users', logs(`${ACTIVITIES} @usr.email:*`, { groupBy: ['@usr.email'] }), { palette: 'purple' }), 4, 3, 4, 3),
      at(toplist('Hosts acted on most', logs(`${ACTIVITIES} @fleetdm.host_display_name:*`, { groupBy: ['@fleetdm.host_display_name'] }), { palette: 'cool' }), 8, 3, 4, 3),
      at(logStream('Logins and failed logins', `${ACTIVITIES} ${anyOf(LOGINS)}`), 0, 6, 6, 4),
      at(logStream('Scripts, software and MDM actions on hosts', `${ACTIVITIES} ${anyOf(ACTIONS)}`), 6, 6, 6, 4),
      at(logStream('Policies, queries and configuration changed', `${ACTIVITIES} ${anyOf(CHANGES)}`), 0, 10, 6, 4),
      at(logStream('Warnings and errors', `${ACTIVITIES} status:(warn OR error)`), 6, 10, 6, 4),
    ],
    37,
    14,
  );
  return {
    title: 'FleetDM Overview',
    description: 'Hosts of a Fleet server (status, platforms, OS versions, disk, failing policies, vulnerabilities, MDM), its global policies and its activity feed.',
    layout_type: 'ordered',
    template_variables: [],
    widgets: [intro, overview, hosts, compliance, activity],
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

/** Our logo tile instead of a banner image (there is none for Fleet), as the integrations-core dashboards have. */
function withLogo(widgets) {
  return widgets.map((w) => {
    if (w.definition.type === 'group') return { ...w, definition: { ...w.definition, widgets: withLogo(w.definition.widgets) } };
    if (w.definition.type === 'image') return { ...w, definition: { type: 'image', title: 'FleetDM', logo: 'fleetdm', background: '#ffffff' } };
    return w;
  });
}

const renamed = (widgets) =>
  widgets.map((w) => ({ ...w, id: w.id.replace(/^upstream-/, 'fleetdm-'), definition: w.definition.type === 'group' ? { ...w.definition, widgets: renamed(w.definition.widgets) } : w.definition }));

export function buildDashboard() {
  seq = 0;
  const doc = dashboard();
  const converted = convertDashboard(doc, 'fleetdm', 'fleetdm_overview.json', 'observecraft');
  return {
    ...converted,
    id: 'fleetdm-overview',
    description: doc.description,
    widgets: renamed(withLogo(converted.widgets)),
    source: { repository: 'observecraft', path: 'scripts/fleetdm-dashboard.mjs', license: 'MIT' },
  };
}

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
