// The NetBird dashboard. Datadog has no NetBird integration, so this is ObserveCraft's own, over what the NetBird
// connector package collects (integrations/connectors/netbird): the peers' `netbird.peer.*` gauges and the audit
// events, logs with `source:netbird` (`@evt.name` the activity code, `@netbird.category` its first segment, `@usr.*`
// the initiator). Written in the Datadog widget format, as the package's asset; `scripts/connector.mjs build` converts
// it like the integration dashboards (import-integration-dashboards.mjs) into the package's dist/dashboards.json.
// node scripts/netbird-dashboard.mjs   (writes integrations/connectors/netbird/assets/dashboards/overview.json)
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// ---------------------------------------------------------------- queries (Datadog format)

const metric = (query, aggregator = 'last', name = 'query1') => ({ data_source: 'metrics', name, query, aggregator });

/** A log query: count, or the cardinality of a facet; grouped by facets. */
const logs = (search, { name = 'query1', groupBy = [], limit = 10, aggregation = 'count', metric: facet } = {}) => ({
  data_source: 'logs',
  name,
  search: { query: search },
  indexes: ['*'],
  compute: { aggregation, ...(facet ? { metric: facet } : {}) },
  group_by: groupBy.map((f) => ({ facet: f, limit, sort: { aggregation: 'count', order: 'desc' } })),
  storage: 'hot',
});

// ---------------------------------------------------------------- widgets (Datadog format)

const head = (title) => ({ title, title_size: '16', title_align: 'left' });

function queryValue(title, queries, { formula = 'query1', formats = [], timeseries = false } = {}) {
  return {
    type: 'query_value',
    ...head(title),
    requests: [{ response_format: 'scalar', queries, formulas: [{ formula }], conditional_formats: formats }],
    autoscale: true,
    precision: 0,
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

function toplist(title, query, { count = 10, palette } = {}) {
  return {
    type: 'toplist',
    ...head(title),
    requests: [
      {
        response_format: 'scalar',
        queries: [query],
        formulas: [{ formula: 'query1' }],
        sort: { count, order_by: [{ type: 'formula', index: 0, order: 'desc' }] },
        ...(palette ? { style: { palette } } : {}),
      },
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

/** A scalar widget over one query, its groups drawn as the type does (geomap, sunburst, treemap, bar_chart). */
function scalar(type, title, query, extra = {}) {
  return {
    type,
    ...head(title),
    requests: [{ response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }], sort: { count: 20, order_by: [{ type: 'formula', index: 0, order: 'desc' }] } }],
    ...extra,
  };
}

const geomap = (title, query) => scalar('geomap', title, query, { style: { palette: 'hostmap_blues', palette_flip: false }, view: { focus: 'WORLD' } });
const sunburst = (title, query) => scalar('sunburst', title, query, { legend: { type: 'inline' } });
const treemap = (title, query) => scalar('treemap', title, query);
const barChart = (title, query, palette = 'classic') => scalar('bar_chart', title, query, { style: { palette } });

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

// Filled tiles: green when all is well, yellow or red when something needs a look.
const GOOD = [{ comparator: '>=', value: 0, palette: 'white_on_green' }];
const BAD = [
  { comparator: '>', value: 0, palette: 'white_on_red' },
  { comparator: '<=', value: 0, palette: 'white_on_green' },
];
const WARN = [
  { comparator: '>', value: 0, palette: 'white_on_yellow' },
  { comparator: '<=', value: 0, palette: 'white_on_green' },
];
const NONE = [{ comparator: '>=', value: 0, palette: 'white_on_gray' }];

// ---------------------------------------------------------------- layout

let seq = 0;
const at = (definition, x, y, width, height) => ({ id: ++seq, definition, layout: { x, y, width, height } });
const group = (title, color, widgets, y, height, extra = {}) => at({ type: 'group', title, background_color: color, layout_type: 'ordered', widgets, ...extra }, 0, y, 12, height);

const PEERS = 'netbird.peer';
const EVENTS = 'source:netbird';

function dashboard() {
  const intro = group(
    '',
    'vivid_blue',
    [
      at(
        note(
          '**NetBird** connects devices, servers and cloud resources over a WireGuard® overlay network. This dashboard shows the peers of the connected NetBird account (connected or not, last seen, expired logins, client versions) and its audit log: who added or removed peers, changed policies, routes, DNS and networks, created setup keys or invited users.\n\nConnect an account in [Integrations → NetBird](/integrations/netbird): ObserveCraft reads its management API every minute, for NetBird Cloud and self-hosted NetBird alike.',
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
    { show_title: false, banner_img: '/images/integrations/netbird-banner.svg' },
  );
  const overview = group(
    'Overview',
    'blue',
    [
      at(queryValue('Peers', [metric(`sum:${PEERS}.count{*}`)], { formats: NONE }), 0, 0, 2, 2),
      at(queryValue('Connected', [metric(`sum:${PEERS}.connected{*}`)], { formats: GOOD }), 2, 0, 2, 2),
      at(queryValue('Disconnected', [metric(`sum:${PEERS}.count{*}`), metric(`sum:${PEERS}.connected{*}`, 'last', 'query2')], { formula: 'query1 - query2', formats: WARN }), 4, 0, 2, 2),
      at(queryValue('Login expired', [metric(`sum:${PEERS}.login_expired{*}`)], { formats: BAD }), 6, 0, 2, 2),
      at(queryValue('Waiting for approval', [metric(`sum:${PEERS}.approval_required{*}`)], { formats: WARN }), 8, 0, 2, 2),
      at(queryValue('Audit events', [logs(EVENTS)], { formats: NONE, timeseries: true }), 10, 0, 2, 2),
    ],
    5,
    3,
  );
  const peers = group(
    'Peers',
    'green',
    [
      at(geomap('Peers around the world', metric(`sum:${PEERS}.count{*} by {country_code}`)), 0, 0, 8, 4),
      at(sunburst('Peers by OS', metric(`sum:${PEERS}.count{*} by {os}`)), 8, 0, 4, 4),
      at(sunburst('Client versions', metric(`sum:${PEERS}.count{*} by {netbird_version}`)), 0, 4, 4, 3),
      at(treemap('Peers by city', metric(`sum:${PEERS}.count{*} by {city}`)), 4, 4, 4, 3),
      at(barChart('Connected peers by OS', metric(`sum:${PEERS}.connected{*} by {os}`)), 8, 4, 4, 3),
      at(
        timeseries('Connected peers', [metric(`sum:${PEERS}.connected{*}`, 'avg', 'query1'), metric(`sum:${PEERS}.count{*}`, 'avg', 'query2')], {
          formulas: [
            { alias: 'connected', formula: 'query1' },
            { alias: 'peers', formula: 'query2' },
          ],
          display: 'area',
          palette: 'cool',
        }),
        0,
        7,
        6,
        3,
      ),
      at(timeseries('Connected peers by client version', [metric(`sum:${PEERS}.connected{*} by {netbird_version}`, 'avg')], { display: 'bars', palette: 'classic' }), 6, 7, 6, 3),
      at(toplist('Longest unseen peers', metric(`max:${PEERS}.last_seen_age{*} by {peer}`), { palette: 'warm' }), 0, 10, 4, 4),
      at(
        table('Peers', [
          ['connected', `max:${PEERS}.connected{*} by {peer,os,netbird_version}`],
          ['seconds since seen', `max:${PEERS}.last_seen_age{*} by {peer,os,netbird_version}`],
          ['login expired', `max:${PEERS}.login_expired{*} by {peer,os,netbird_version}`],
          ['reachable peers', `max:${PEERS}.accessible_peers{*} by {peer,os,netbird_version}`],
        ]),
        4,
        10,
        8,
        4,
      ),
    ],
    8,
    14,
  );
  const audit = group(
    'Audit log',
    'purple',
    [
      at(timeseries('Events over time', [logs(EVENTS, { groupBy: ['@netbird.category'] })], { display: 'bars', palette: 'classic' }), 0, 0, 8, 3),
      at(sunburst('Events by category', logs(EVENTS, { groupBy: ['@netbird.category'] })), 8, 0, 4, 3),
      at(barChart('Top activities', logs(EVENTS, { groupBy: ['@evt.name'] })), 0, 3, 4, 3),
      at(toplist('Most active users', logs(`${EVENTS} @usr.email:*`, { groupBy: ['@usr.email'] }), { palette: 'purple' }), 4, 3, 4, 3),
      at(queryValue('Warnings and errors', [logs(`${EVENTS} status:(warn OR error)`)], { formats: BAD, timeseries: true }), 8, 3, 4, 3),
      at(
        logStream('Peers added, removed and expired', `${EVENTS} @evt.name:(peer.user.add OR peer.setupkey.add OR user.peer.delete OR peer.login.expire OR peer.approve OR peer.approval.revoke)`),
        0,
        6,
        6,
        4,
      ),
      at(logStream('Access changes: policies, routes, networks, DNS', `${EVENTS} @netbird.category:(policy OR rule OR route OR network OR nameserver OR dns OR posture)`), 6, 6, 6, 4),
      at(logStream('Users, setup keys and tokens', `${EVENTS} @netbird.category:(user OR setupkey OR personal OR service OR account)`), 0, 10, 6, 4),
      at(logStream('Warnings and errors', `${EVENTS} status:(warn OR error)`), 6, 10, 6, 4),
    ],
    22,
    14,
  );
  return {
    title: 'NetBird Overview',
    description: 'Peers of a NetBird account (connected, last seen, expired logins, OS, client versions) and its audit log.',
    layout_type: 'ordered',
    template_variables: [],
    widgets: [intro, overview, peers, audit],
  };
}

/** The dashboard in Datadog's format, as the package ships it. */
export function datadogDashboard() {
  seq = 0;
  return dashboard();
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dir = new URL('../integrations/connectors/netbird/assets/dashboards/', import.meta.url);
  mkdirSync(dir, { recursive: true });
  writeFileSync(new URL('overview.json', dir), JSON.stringify(datadogDashboard(), null, 2) + '\n');
  console.log('wrote integrations/connectors/netbird/assets/dashboards/overview.json (then: node scripts/connector.mjs build integrations/connectors/netbird)');
}
