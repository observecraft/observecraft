// Mock `observecraft.*` self-monitoring metrics (server/crates/oc-api/src/self_monitoring.rs): what the server measures
// about the install every minute, with the tags it puts on them, for the ObserveCraft Overview dashboard. Fictional
// install: two ClickHouse replicas, the demo's organizations.
import type { CloudMetricDef } from './cloudMetrics';

type Tags = Record<string, string>;

const GIB = 2 ** 30;
const REPLICAS = ['chi-observecraft-0-0', 'chi-observecraft-0-1'];
/** table, GiB on disk, rows (millions), seconds since its last write */
const TABLES: [string, number, number, number][] = [
  ['logs', 412, 2_900, 4],
  ['otel_traces', 268, 1_450, 6],
  ['metrics_samples', 151, 9_800, 3],
  ['otel_rum', 37, 210, 21],
  ['otel_events', 4, 12, 95],
  ['profiles', 22, 3, 48],
];
const TABLE = new Map(TABLES.map(([t, gib, rows, age]) => [t, { gib, rows, age }]));
const ORGS: [string, number][] = [
  ['example-org', 1],
  ['acme-demo', 0.42],
  ['storefront-staging', 0.17],
  ['data-platform', 0.08],
];
/** signal, events per hour of the largest org, bytes per event on disk */
const SIGNALS: [string, number, number][] = [
  ['logs', 9_200_000, 140],
  ['spans', 4_600_000, 180],
  ['metric_points', 31_000_000, 16],
  ['rum', 650_000, 170],
  ['events', 40_000, 300],
  ['profiles', 9_000, 2_400],
];
/** component, resource, USD per month */
const COSTS: [string, string, number][] = [
  ['clickhouse', 'cpu', 182],
  ['clickhouse', 'memory', 96],
  ['clickhouse', 'storage', 71],
  ['kafka', 'cpu', 61],
  ['kafka', 'memory', 34],
  ['kafka', 'storage', 18],
  ['server', 'cpu', 88],
  ['server', 'memory', 41],
  ['intake', 'cpu', 39],
  ['intake', 'memory', 12],
  ['collector', 'cpu', 27],
  ['collector', 'memory', 15],
  ['postgres', 'cpu', 14],
  ['postgres', 'memory', 9],
  ['postgres', 'storage', 2],
  ['valkey', 'cpu', 6],
  ['valkey', 'memory', 8],
];
const COST = new Map(COSTS.map(([c, r, usd]) => [`${c}/${r}`, usd]));

export const OBSERVECRAFT_SETS: Record<string, Tags[]> = {
  observecraft_install: [{}],
  observecraft_servers: [{ server: 'observecraft-server-7c9d5b6f4-k2x8p' }, { server: 'observecraft-server-7c9d5b6f4-q7m3t' }],
  observecraft_disks: REPLICAS.map((replica) => ({ replica, disk: 'default' })),
  observecraft_replicas: REPLICAS.map((replica) => ({ replica })),
  observecraft_tables: TABLES.map(([table]) => ({ table })),
  observecraft_topics: [
    { topic: 'oc.logs.json', consumer: 'log-ingest' },
    { topic: 'oc.otlp.logs', consumer: 'log-ingest' },
  ],
  observecraft_jobs: ['aws', 'azure', 'gcp', 'netbird', 'fleetdm'].map((provider) => ({ provider })),
  observecraft_costs: COSTS.map(([component, resource]) => ({ component, resource })),
  observecraft_usage: ORGS.flatMap(([org]) => SIGNALS.map(([signal]) => ({ org, signal }))),
};

const DISK_TOTAL = 1_000 * GIB;
const DISK_USED = (e: Tags) => (e.replica?.endsWith('-0') ? 0.71 : 0.68) * DISK_TOTAL;
const GROWTH = (e: Tags) => (e.replica?.endsWith('-0') ? 11.5 : 10.8) * GIB;
const share = (e: Tags) => ORGS.find(([o]) => o === e.org)?.[1] ?? 0;
const signal = (e: Tags) => SIGNALS.find(([s]) => s === e.signal);

const install = (description: string, unit: string | undefined, base: number, noise = 0.05, integer = false): CloudMetricDef => ({
  set: 'observecraft_install',
  description,
  unit,
  base: () => base,
  noise,
  integer,
});
const MONTHLY = COSTS.reduce((sum, [, , usd]) => sum + usd, 0);

export const OBSERVECRAFT_METRICS: Record<string, CloudMetricDef> = {
  'observecraft.up': { set: 'observecraft_servers', description: 'ObserveCraft server is running', base: () => 1, noise: 0, integer: true },
  'observecraft.clickhouse.reachable': install('ClickHouse answers the server', undefined, 1, 0, true),
  'observecraft.clickhouse.ping_ms': install('ClickHouse query round trip', 'millisecond', 3.2, 0.4),
  'observecraft.clickhouse.disk.total_bytes': { set: 'observecraft_disks', description: 'ClickHouse disk size', unit: 'byte', base: () => DISK_TOTAL, noise: 0 },
  'observecraft.clickhouse.disk.used_bytes': { set: 'observecraft_disks', description: 'ClickHouse disk space used', unit: 'byte', base: DISK_USED, noise: 0.01 },
  'observecraft.clickhouse.disk.free_bytes': { set: 'observecraft_disks', description: 'ClickHouse disk space free', unit: 'byte', base: (e) => DISK_TOTAL - DISK_USED(e), noise: 0.02 },
  'observecraft.clickhouse.disk.used_pct': {
    set: 'observecraft_disks',
    description: 'ClickHouse disk space used',
    unit: 'percent',
    base: (e) => (DISK_USED(e) / DISK_TOTAL) * 100,
    noise: 0.01,
    max: 100,
  },
  'observecraft.clickhouse.disk.growth_bytes_per_day': { set: 'observecraft_replicas', description: 'ClickHouse disk growth per day', unit: 'byte', base: GROWTH, noise: 0.08 },
  'observecraft.clickhouse.disk.days_until_full': {
    set: 'observecraft_replicas',
    description: 'Days until the ClickHouse disk is full at its growth',
    unit: 'day',
    base: (e) => (DISK_TOTAL - DISK_USED(e)) / GROWTH(e),
    noise: 0.05,
  },
  'observecraft.clickhouse.table.bytes': { set: 'observecraft_tables', description: 'ClickHouse table size on disk', unit: 'byte', base: (e) => (TABLE.get(e.table)?.gib ?? 0) * GIB, noise: 0.01 },
  'observecraft.clickhouse.table.rows': {
    set: 'observecraft_tables',
    description: 'ClickHouse table rows',
    unit: 'row',
    base: (e) => (TABLE.get(e.table)?.rows ?? 0) * 1e6,
    noise: 0.01,
    integer: true,
  },
  'observecraft.clickhouse.table.last_write_age_seconds': {
    set: 'observecraft_tables',
    description: 'Seconds since the table was last written',
    unit: 'second',
    base: (e) => TABLE.get(e.table)?.age ?? 0,
    noise: 0.6,
    integer: true,
  },
  'observecraft.clickhouse.table.parts_max_per_partition': {
    set: 'observecraft_tables',
    description: 'Most active parts in one partition of the table',
    unit: 'item',
    base: (e) => 6 + ((TABLE.get(e.table)?.rows ?? 0) % 37),
    noise: 0.3,
    integer: true,
  },
  'observecraft.clickhouse.mutations.pending': install('ClickHouse mutations not done yet', 'operation', 1, 1, true),
  'observecraft.clickhouse.mutations.failed': install('ClickHouse mutations that failed', 'operation', 0, 0, true),
  'observecraft.clickhouse.replication.max_delay_seconds': install('Largest replication delay of a ClickHouse replica', 'second', 1.5, 0.8),
  'observecraft.clickhouse.replication.queue_size': install('ClickHouse replication tasks queued', 'task', 4, 0.8, true),
  'observecraft.kafka.consumer_lag': {
    set: 'observecraft_topics',
    description: 'Records not yet read by the consumer',
    unit: 'record',
    base: (e) => (e.topic === 'oc.logs.json' ? 1_800 : 450),
    noise: 0.9,
    integer: true,
  },
  'observecraft.postgres.connections.used': install('Postgres connections in use', 'connection', 38, 0.15, true),
  'observecraft.postgres.connections.max': install('Postgres connection limit', 'connection', 200, 0, true),
  'observecraft.postgres.connections.used_pct': install('Postgres connections in use', 'percent', 19, 0.15),
  'observecraft.postgres.database.bytes': install('Postgres database size', 'byte', 3.4 * GIB, 0.002),
  'observecraft.jobs.overdue': { set: 'observecraft_jobs', description: 'Crawl jobs past their next run', unit: 'job', base: () => 0, noise: 0, integer: true },
  'observecraft.jobs.failing': { set: 'observecraft_jobs', description: 'Crawl jobs whose last run failed', unit: 'job', base: (e) => (e.provider === 'azure' ? 1 : 0), noise: 0, integer: true },
  'observecraft.log_metrics.outbox_backlog': install('Log-based metric batches waiting to be written', 'item', 2, 1, true),
  'observecraft.cost.estimated_monthly': {
    set: 'observecraft_costs',
    description: 'Estimated cost per month at the current usage',
    unit: 'dollar',
    base: (e) => COST.get(`${e.component}/${e.resource}`) ?? 0,
    noise: 0.04,
  },
  'observecraft.cost.projected_monthly': install('Projected cost of the next 30 days, storage growth included', 'dollar', MONTHLY * 1.06, 0.02),
  'observecraft.usage.events': {
    set: 'observecraft_usage',
    description: 'Events stored in the previous hour',
    unit: 'event',
    base: (e) => share(e) * (signal(e)?.[1] ?? 0),
    noise: 0.25,
    integer: true,
  },
  'observecraft.usage.estimated_bytes': {
    set: 'observecraft_usage',
    description: 'Estimated storage of the events stored in the previous hour',
    unit: 'byte',
    base: (e) => share(e) * (signal(e)?.[1] ?? 0) * (signal(e)?.[2] ?? 0),
    noise: 0.25,
  },
};

/** ObserveCraft's own pods, as the Datadog Agent's container check tags them (its namespace and container names). */
export const OBSERVECRAFT_PODS: Tags[] = ['server', 'server', 'intake', 'collector', 'clickhouse', 'clickhouse', 'kafka', 'postgres', 'valkey'].map((name, i) => ({
  container_name: name,
  kube_container_name: name,
  kube_namespace: 'observecraft',
  kube_cluster_name: 'observecraft-prod',
  host: `i-0a1b2c3d4e5f6001${i}`,
  short_image: name === 'server' || name === 'intake' ? `observecraft-${name}` : name,
  pod_name: `observecraft-${name}-${i.toString(36)}`,
}));
