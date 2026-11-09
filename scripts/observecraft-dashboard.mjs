// The ObserveCraft Overview dashboard: ObserveCraft watching itself. The server's self-monitoring job
// (server/crates/oc-api/src/self_monitoring.rs) writes `observecraft.*` gauges every minute into the operator's org:
// ClickHouse (reachable, disks, tables, parts, mutations, replication), log ingestion's Kafka lag, Postgres, the
// background jobs, usage per organization and the estimated cost. Pods come from the Datadog Agent when it runs in
// ObserveCraft's namespace (`kube_namespace:observecraft`). Written in the Datadog widget format the integration
// dashboards use and converted like them (import-integration-dashboards.mjs).
// node scripts/observecraft-dashboard.mjs   (writes server/seeds/integrations/observecraft/{dashboards.json,units.csv})
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { convertDashboard } from './import-integration-dashboards.mjs';

// ---------------------------------------------------------------- queries (Datadog format)

const metric = (query, aggregator = 'last', name = 'query1') => ({ data_source: 'metrics', name, query, aggregator });

// ---------------------------------------------------------------- widgets (Datadog format)

const head = (title) => ({ title, title_size: '16', title_align: 'left' });

function queryValue(title, queries, { formula = 'query1', formats = [], timeseries = false, precision = 0 } = {}) {
  return {
    type: 'query_value',
    ...head(title),
    requests: [{ response_format: 'scalar', queries, formulas: [{ formula }], conditional_formats: formats }],
    autoscale: true,
    precision,
    ...(timeseries ? { timeseries_background: { type: 'area' } } : {}),
  };
}

function timeseries(title, queries, { formulas, display = 'line', palette = 'dog_classic' } = {}) {
  return {
    type: 'timeseries',
    ...head(title),
    show_legend: true,
    legend_layout: 'auto',
    requests: [
      {
        response_format: 'timeseries',
        queries,
        formulas: formulas ?? queries.map((q) => ({ formula: q.name, ...(/ by \{/.test(q.query) ? {} : { alias: title.toLowerCase() }) })),
        style: { palette, order_by: 'values' },
        display_type: display,
      },
    ],
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

/** A scalar widget over one query, its groups drawn as the type does (sunburst, treemap). */
function scalar(type, title, query, extra = {}) {
  return {
    type,
    ...head(title),
    requests: [{ response_format: 'scalar', queries: [query], formulas: [{ formula: 'query1' }], sort: { count: 20, order_by: [{ type: 'formula', index: 0, order: 'desc' }] } }],
    ...extra,
  };
}

const sunburst = (title, query) => scalar('sunburst', title, query, { legend: { type: 'inline' } });
const treemap = (title, query) => scalar('treemap', title, query);

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

// Filled tiles: green when all is well, yellow or red when something needs a look (the first matching format wins).
const above = (warn, crit) => [
  { comparator: '>=', value: crit, palette: 'white_on_red' },
  { comparator: '>=', value: warn, palette: 'white_on_yellow' },
  { comparator: '<', value: warn, palette: 'white_on_green' },
];
const below = (warn, crit) => [
  { comparator: '<', value: crit, palette: 'white_on_red' },
  { comparator: '<', value: warn, palette: 'white_on_yellow' },
  { comparator: '>=', value: warn, palette: 'white_on_green' },
];
const BAD = [
  { comparator: '>', value: 0, palette: 'white_on_red' },
  { comparator: '<=', value: 0, palette: 'white_on_green' },
];
const WARN = [
  { comparator: '>', value: 0, palette: 'white_on_yellow' },
  { comparator: '<=', value: 0, palette: 'white_on_green' },
];
const UP = [
  { comparator: '<', value: 1, palette: 'white_on_red' },
  { comparator: '>=', value: 1, palette: 'white_on_green' },
];
const NONE = [{ comparator: '>=', value: 0, palette: 'white_on_gray' }];

// ---------------------------------------------------------------- layout

let seq = 0;
const at = (definition, x, y, width, height) => ({ id: ++seq, definition, layout: { x, y, width, height } });
const group = (title, color, widgets, y, height, extra = {}) => at({ type: 'group', title, background_color: color, layout_type: 'ordered', widgets, ...extra }, 0, y, 12, height);

const OC = 'observecraft';
const CH = `${OC}.clickhouse`;
/** The pods of ObserveCraft as the Datadog Agent reports them. */
const PODS = '{kube_namespace:observecraft}';

function dashboard() {
  const intro = group(
    '',
    'vivid_blue',
    [
      at(note('', 'white'), 0, 0, 2, 2),
      at(
        note(
          "**ObserveCraft watching itself.** Every minute the server measures what keeps this install healthy: ClickHouse (reachable, disk space and its growth, tables, parts, mutations, replication), how far log ingestion is behind Kafka, Postgres connections, the background jobs, what each organization stores and what the install costs. Its **[ObserveCraft] monitors** alert before something breaks; set `OC_HEARTBEAT_URL` to also hear when ObserveCraft itself is down.\n\nPods, CPU and memory come from the Datadog Agent when it runs in ObserveCraft's namespace. Costs are estimates: the Kubernetes prices in [Kubernetes → Autoscaling settings](/kubernetes/autoscaling) and `OC_SELF_MONITORING_STORAGE_GB_MONTH` (Helm `selfMonitoring.storageGbMonth`) per GB-month.",
          'white',
        ),
        2,
        0,
        10,
        2,
      ),
    ],
    0,
    3,
    { show_title: false },
  );
  const health = group(
    'Health at a glance',
    'blue',
    [
      at(queryValue('ClickHouse up', [metric(`min:${CH}.reachable{*}`)], { formats: UP }), 0, 0, 2, 2),
      at(queryValue('Disk used', [metric(`max:${CH}.disk.used_pct{*}`)], { formats: above(75, 85), precision: 1 }), 2, 0, 2, 2),
      at(queryValue('Disk full in', [metric(`min:${CH}.disk.days_until_full{*}`)], { formats: below(14, 7), precision: 0 }), 4, 0, 2, 2),
      at(queryValue('Log lag', [metric(`sum:${OC}.kafka.consumer_lag{consumer:log-ingest}`)], { formats: above(200000, 1000000), timeseries: true }), 6, 0, 2, 2),
      at(queryValue('Postgres conns', [metric(`max:${OC}.postgres.connections.used_pct{*}`)], { formats: above(70, 85), precision: 1 }), 8, 0, 2, 2),
      at(queryValue('Failed mutations', [metric(`sum:${CH}.mutations.failed{*}`)], { formats: BAD }), 10, 0, 2, 2),
      at(queryValue('Overdue jobs', [metric(`sum:${OC}.jobs.overdue{*}`)], { formats: WARN }), 0, 2, 2, 2),
      at(queryValue('Failing jobs', [metric(`sum:${OC}.jobs.failing{*}`)], { formats: WARN }), 2, 2, 2, 2),
      at(queryValue('Oldest write', [metric(`max:${CH}.table.last_write_age_seconds{*}`)], { formats: above(300, 900) }), 4, 2, 2, 2),
      at(queryValue('Max parts', [metric(`max:${CH}.table.parts_max_per_partition{*}`)], { formats: above(300, 600) }), 6, 2, 2, 2),
      at(queryValue('Cost / month', [metric(`sum:${OC}.cost.estimated_monthly{*}`)], { formats: NONE }), 8, 2, 2, 2),
      at(queryValue('Next 30 days', [metric(`sum:${OC}.cost.projected_monthly{*}`)], { formats: NONE }), 10, 2, 2, 2),
    ],
    4,
    5,
  );
  const ingestion = group(
    'Ingestion',
    'green',
    [
      at(timeseries('Seconds since each table was last written', [metric(`max:${CH}.table.last_write_age_seconds{*} by {table}`, 'avg')], { palette: 'warm' }), 0, 0, 6, 3),
      at(timeseries('Log ingestion lag behind Kafka (records)', [metric(`sum:${OC}.kafka.consumer_lag{*} by {topic}`, 'avg')], { display: 'area', palette: 'orange' }), 6, 0, 6, 3),
      at(timeseries('Events stored per hour, by signal', [metric(`sum:${OC}.usage.events{*} by {signal}`, 'avg')], { display: 'bars', palette: 'classic' }), 0, 3, 8, 3),
      at(sunburst('Events by signal', metric(`sum:${OC}.usage.events{*} by {signal}`, 'sum')), 8, 3, 4, 3),
    ],
    9,
    7,
  );
  const clickhouse = group(
    'ClickHouse',
    'purple',
    [
      at(
        timeseries('Disk used and total', [metric(`sum:${CH}.disk.used_bytes{*}`, 'avg', 'query1'), metric(`sum:${CH}.disk.total_bytes{*}`, 'avg', 'query2')], {
          formulas: [
            { alias: 'used', formula: 'query1' },
            { alias: 'total', formula: 'query2' },
          ],
          display: 'area',
          palette: 'purple',
        }),
        0,
        0,
        6,
        3,
      ),
      at(timeseries('Disk used (%) by replica', [metric(`max:${CH}.disk.used_pct{*} by {replica,disk}`, 'avg')], { palette: 'warm' }), 6, 0, 6, 3),
      at(timeseries('Disk growth per day', [metric(`sum:${CH}.disk.growth_bytes_per_day{*} by {replica}`, 'avg')], { display: 'bars', palette: 'purple' }), 0, 3, 6, 3),
      at(toplist('Days until the disk is full', metric(`min:${CH}.disk.days_until_full{*} by {replica}`), { palette: 'warm' }), 6, 3, 6, 3),
      at(treemap('Table sizes', metric(`sum:${CH}.table.bytes{*} by {table}`)), 0, 6, 6, 4),
      at(
        table('Tables', [
          ['bytes', `sum:${CH}.table.bytes{*} by {table}`],
          ['rows', `sum:${CH}.table.rows{*} by {table}`],
          ['seconds since last write', `max:${CH}.table.last_write_age_seconds{*} by {table}`],
          ['most parts in a partition', `max:${CH}.table.parts_max_per_partition{*} by {table}`],
        ]),
        6,
        6,
        6,
        4,
      ),
      at(timeseries('Most parts in a partition', [metric(`max:${CH}.table.parts_max_per_partition{*} by {table}`, 'max')], { palette: 'orange' }), 0, 10, 4, 3),
      at(
        timeseries('Mutations', [metric(`sum:${CH}.mutations.pending{*}`, 'avg', 'query1'), metric(`sum:${CH}.mutations.failed{*}`, 'avg', 'query2')], {
          formulas: [
            { alias: 'pending', formula: 'query1' },
            { alias: 'failed', formula: 'query2' },
          ],
          palette: 'warm',
        }),
        4,
        10,
        4,
        3,
      ),
      at(
        timeseries('Replication (clusters)', [metric(`max:${CH}.replication.max_delay_seconds{*}`, 'avg', 'query1'), metric(`sum:${CH}.replication.queue_size{*}`, 'avg', 'query2')], {
          formulas: [
            { alias: 'delay (s)', formula: 'query1' },
            { alias: 'queue', formula: 'query2' },
          ],
          palette: 'cool',
        }),
        8,
        10,
        4,
        3,
      ),
      at(timeseries('Query round trip (ms)', [metric(`max:${CH}.ping_ms{*}`, 'avg')], { palette: 'cool' }), 0, 13, 12, 2),
    ],
    16,
    16,
  );
  const postgres = group(
    'Postgres and background jobs',
    'gray',
    [
      at(
        timeseries('Postgres connections', [metric(`max:${OC}.postgres.connections.used{*}`, 'avg', 'query1'), metric(`max:${OC}.postgres.connections.max{*}`, 'avg', 'query2')], {
          formulas: [
            { alias: 'used', formula: 'query1' },
            { alias: 'limit', formula: 'query2' },
          ],
          palette: 'cool',
        }),
        0,
        0,
        4,
        3,
      ),
      at(timeseries('Postgres database size', [metric(`max:${OC}.postgres.database.bytes{*}`, 'avg')], { display: 'area', palette: 'cool' }), 4, 0, 4, 3),
      at(timeseries('Log-based metrics waiting to be written', [metric(`max:${OC}.log_metrics.outbox_backlog{*}`, 'avg')], { palette: 'orange' }), 8, 0, 4, 3),
      at(timeseries('Overdue crawl jobs by provider', [metric(`sum:${OC}.jobs.overdue{*} by {provider}`, 'avg')], { display: 'bars', palette: 'warm' }), 0, 3, 6, 3),
      at(timeseries('Failing crawl jobs by provider', [metric(`sum:${OC}.jobs.failing{*} by {provider}`, 'avg')], { display: 'bars', palette: 'warm' }), 6, 3, 6, 3),
    ],
    32,
    7,
  );
  const usage = group(
    'Usage per organization',
    'orange',
    [
      at(toplist('Events stored in the last hour, by organization', metric(`sum:${OC}.usage.events{*} by {org}`)), 0, 0, 6, 4),
      at(toplist('Estimated storage, by organization', metric(`sum:${OC}.usage.estimated_bytes{*} by {org}`)), 6, 0, 6, 4),
      at(timeseries('Events per hour, by organization', [metric(`sum:${OC}.usage.events{*} by {org}`, 'avg')], { display: 'bars', palette: 'classic' }), 0, 4, 8, 3),
      at(sunburst('Storage by organization and signal', metric(`sum:${OC}.usage.estimated_bytes{*} by {org,signal}`)), 8, 4, 4, 3),
      at(
        table('Organizations', [
          ['events (last hour)', `sum:${OC}.usage.events{*} by {org,signal}`],
          ['estimated bytes', `sum:${OC}.usage.estimated_bytes{*} by {org,signal}`],
        ]),
        0,
        7,
        12,
        4,
      ),
    ],
    39,
    12,
  );
  const cost = group(
    'Cost and projection',
    'yellow',
    [
      at(queryValue('Estimated (USD/month)', [metric(`sum:${OC}.cost.estimated_monthly{*}`)], { formats: NONE, timeseries: true }), 0, 0, 3, 3),
      at(queryValue('Projected next 30 days (USD)', [metric(`sum:${OC}.cost.projected_monthly{*}`)], { formats: NONE, timeseries: true }), 3, 0, 3, 3),
      at(queryValue('Storage (USD/month)', [metric(`sum:${OC}.cost.estimated_monthly{resource:storage}`)], { formats: NONE, timeseries: true }), 6, 0, 3, 3),
      at(queryValue('Disk growth per day', [metric(`sum:${CH}.disk.growth_bytes_per_day{*}`)], { formats: NONE }), 9, 0, 3, 3),
      at(treemap('Estimated cost by component', metric(`sum:${OC}.cost.estimated_monthly{*} by {component}`)), 0, 3, 6, 4),
      at(timeseries('Estimated cost by resource', [metric(`sum:${OC}.cost.estimated_monthly{*} by {resource}`, 'avg')], { display: 'area', palette: 'warm' }), 6, 3, 6, 4),
    ],
    51,
    8,
  );
  const pods = group(
    'Pods (from the Datadog Agent)',
    'vivid_purple',
    [
      at(timeseries('CPU by container', [metric(`sum:container.cpu.usage${PODS} by {kube_container_name}`, 'avg')], { display: 'area', palette: 'cool' }), 0, 0, 6, 3),
      at(timeseries('Memory by container', [metric(`sum:container.memory.usage${PODS} by {kube_container_name}`, 'avg')], { display: 'area', palette: 'purple' }), 6, 0, 6, 3),
      at(toplist('Container restarts', metric(`sum:kubernetes.containers.restarts${PODS} by {kube_container_name}`, 'max'), { palette: 'warm' }), 0, 3, 12, 3),
    ],
    59,
    7,
  );
  return {
    title: 'ObserveCraft Overview',
    description: "ObserveCraft's own health: ClickHouse, log ingestion, Postgres, background jobs, usage per organization and the estimated cost.",
    layout_type: 'ordered',
    template_variables: [],
    widgets: [intro, health, ingestion, clickhouse, postgres, usage, cost, pods],
  };
}

/** Units of the self-monitoring metrics (Datadog unit names): the server writes none. */
export const OBSERVECRAFT_UNITS = {
  'observecraft.clickhouse.ping_ms': 'millisecond',
  'observecraft.clickhouse.disk.total_bytes': 'byte',
  'observecraft.clickhouse.disk.free_bytes': 'byte',
  'observecraft.clickhouse.disk.used_bytes': 'byte',
  'observecraft.clickhouse.disk.used_pct': 'percent',
  'observecraft.clickhouse.disk.growth_bytes_per_day': 'byte',
  'observecraft.clickhouse.disk.days_until_full': 'day',
  'observecraft.clickhouse.table.bytes': 'byte',
  'observecraft.clickhouse.table.rows': 'row',
  'observecraft.clickhouse.table.last_write_age_seconds': 'second',
  'observecraft.clickhouse.table.parts_max_per_partition': 'item',
  'observecraft.clickhouse.mutations.pending': 'operation',
  'observecraft.clickhouse.mutations.failed': 'operation',
  'observecraft.clickhouse.replication.max_delay_seconds': 'second',
  'observecraft.clickhouse.replication.queue_size': 'task',
  'observecraft.kafka.consumer_lag': 'record',
  'observecraft.postgres.connections.used': 'connection',
  'observecraft.postgres.connections.max': 'connection',
  'observecraft.postgres.connections.used_pct': 'percent',
  'observecraft.postgres.database.bytes': 'byte',
  'observecraft.jobs.overdue': 'job',
  'observecraft.jobs.failing': 'job',
  'observecraft.log_metrics.outbox_backlog': 'item',
  'observecraft.cost.estimated_monthly': 'dollar',
  'observecraft.cost.projected_monthly': 'dollar',
  'observecraft.usage.events': 'event',
  'observecraft.usage.estimated_bytes': 'byte',
};

/** The intro's first note becomes our logo tile, as the integrations-core dashboards have theirs. */
function withLogo(widgets) {
  let placed = false;
  return widgets.map((w) => {
    if (w.definition.type === 'group') return { ...w, definition: { ...w.definition, widgets: withLogo(w.definition.widgets) } };
    if (!placed && w.definition.type === 'note' && !w.definition.content) {
      placed = true;
      return { ...w, definition: { type: 'image', title: 'ObserveCraft', logo: 'observecraft', background: '#ffffff' } };
    }
    return w;
  });
}

const renamed = (widgets) =>
  widgets.map((w) => ({
    ...w,
    id: w.id.replace(/^upstream-/, 'observecraft-'),
    definition: w.definition.type === 'group' ? { ...w.definition, widgets: renamed(w.definition.widgets) } : w.definition,
  }));

export function buildDashboard() {
  seq = 0;
  const doc = dashboard();
  const converted = convertDashboard(doc, 'observecraft', 'observecraft_overview.json', 'observecraft');
  return {
    ...converted,
    id: 'observecraft-overview',
    description: doc.description,
    widgets: renamed(withLogo(converted.widgets)),
    source: { repository: 'observecraft', path: 'scripts/observecraft-dashboard.mjs', license: 'MIT' },
  };
}

export const unitsCsv = () =>
  `# Units of ObserveCraft's self-monitoring metrics (scripts/observecraft-dashboard.mjs; the server writes none).\n${Object.entries(OBSERVECRAFT_UNITS)
    .map(([name, unit]) => `${name},${unit}`)
    .join('\n')}\n`;

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dir = new URL('../server/seeds/integrations/observecraft/', import.meta.url);
  mkdirSync(dir, { recursive: true });
  writeFileSync(new URL('dashboards.json', dir), JSON.stringify([buildDashboard()], null, 2) + '\n');
  writeFileSync(new URL('units.csv', dir), unitsCsv());
  console.log('wrote server/seeds/integrations/observecraft/{dashboards.json,units.csv}');
}
