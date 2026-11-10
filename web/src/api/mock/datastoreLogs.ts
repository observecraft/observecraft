// Mock logs of the datastores whose integration dashboards the mock shows (PostgreSQL, PgBouncer, Redis, ClickHouse):
// the lines their servers write, as the Datadog Agent tails them (`source:postgresql`, `pgbouncer`, `redis`,
// `clickhouse`) and the integrations' pipelines (pipelines/library.generated.json, run by the mock's engine as the
// server's indexer runs them) parse them. On the hosts the dashboards' mock metrics use (coreMetrics.ts).
import library from '../pipelines/library.generated.json' with { type: 'json' };
import type { Processor } from '../pipelines/types';
import type { LogEvent, LogStatus } from '../types';
import type { Rand } from './generator';
import { runPipeline } from './pipelineEngine';

const HOUR = 3600_000;
const HOSTS = ['i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003', 'i-0a1b2c3d4e5f60006'];

const pad = (n: number, w = 2) => String(n).padStart(w, '0');
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
/** `2026-11-10 08:15:42.120` (UTC). */
const isoDate = (ts: number, sep = '-') => {
  const d = new Date(ts);
  return `${d.getUTCFullYear()}${sep}${pad(d.getUTCMonth() + 1)}${sep}${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())}.${pad(d.getUTCMilliseconds(), 3)}`;
};

/** A line of `source` run through its integration pipeline, as the indexer stores it. */
function parsed(id: string, line: { ts: number; i: number; host: string; service: string; source: string; message: string; tags: Record<string, string> }): LogEvent {
  const p = (library as { id: string; filter: string; processors: Processor[] }[]).find((x) => x.id === id)!;
  const { log } = runPipeline(
    p.filter,
    p.processors,
    { message: line.message, status: 'info', service: line.service, host: line.host, source: line.source, tags: line.tags, attributes: {}, date: null, traceId: '' },
    line.ts,
  );
  return {
    id: `${line.source}-${line.ts}-${line.i}`,
    timestamp: log.date ? Date.parse(log.date) : line.ts,
    status: log.status as LogStatus,
    service: log.service,
    host: log.host,
    source: log.source,
    message: log.message,
    tags: log.tags,
    attributes: log.attributes,
  };
}

/** PostgreSQL with the suggested `log_line_prefix = '%m [%p] %d %a %u %h %c '`: statements, slow queries, errors. */
const PG: [weight: number, line: (r: Rand) => string][] = [
  [6, (r) => `LOG:  duration: ${(0.2 + r() * 40).toFixed(3)} ms  statement: SELECT id, status, total FROM orders WHERE customer_id = ${1000 + Math.floor(r() * 9000)}`],
  [3, (r) => `LOG:  duration: ${(0.5 + r() * 20).toFixed(3)} ms  statement: UPDATE inventory SET reserved = reserved + 1 WHERE sku = 'SKU-${100 + Math.floor(r() * 900)}'`],
  [2, (r) => `LOG:  duration: ${(1200 + r() * 4000).toFixed(3)} ms  statement: SELECT p.id, sum(oi.quantity) FROM order_items oi JOIN products p ON p.id = oi.product_id GROUP BY p.id`],
  [2, () => 'LOG:  statement: BEGIN'],
  [1, () => 'LOG:  checkpoint starting: time'],
  [1, (r) => `LOG:  automatic vacuum of table "orders.public.order_items": index scans: ${1 + Math.floor(r() * 3)}`],
  [1, () => 'ERROR:  duplicate key value violates unique constraint "invoices_number_key"'],
  [1, () => 'ERROR:  canceling statement due to statement timeout'],
  [1, () => 'FATAL:  password authentication failed for user "reporting"'],
  [1, () => 'WARNING:  there is no transaction in progress'],
];
const APPS = ['order-service', 'catalog-service', 'billing', 'inventory-service'];

function postgresLogs(r: Rand, now: number): LogEvent[] {
  const total = PG.reduce((a, [w]) => a + w, 0);
  return Array.from({ length: 480 }, (_, i) => {
    const ts = Math.round(now - Math.pow(r(), 1.3) * 24 * HOUR);
    let n = r() * total;
    const [, body] = PG.find(([w]) => (n -= w) < 0) ?? PG[0];
    const c = Math.floor(r() * HOSTS.length);
    const app = APPS[c];
    const db = ['orders', 'catalog', 'billing', 'inventory'][c];
    const pid = 2000 + Math.floor(r() * 30000);
    const message = `${isoDate(ts)} UTC [${pid}] ${db} ${app} ${app.replace(/-/g, '_')} 10.0.${12 + c}.${20 + Math.floor(r() * 200)} ${Math.floor(ts / 1000).toString(16)}.${pid.toString(16)} ${body(r)}`;
    return parsed('postgres', { ts, i, host: HOSTS[c], service: 'postgresql', source: 'postgresql', message, tags: { env: c === 3 ? 'qa' : 'prod', db } });
  });
}

/** PgBouncer's log: stats every minute, logins, pooler errors. */
const BOUNCER: [weight: number, line: (r: Rand, db: string, app: string) => string][] = [
  [
    4,
    (r) =>
      `LOG stats: ${Math.floor(20 + r() * 400)} xacts/s, ${Math.floor(40 + r() * 900)} queries/s, in ${Math.floor(r() * 90000)} B/s, out ${Math.floor(r() * 400000)} B/s, xact ${Math.floor(300 + r() * 4000)} us, query ${Math.floor(100 + r() * 1500)} us, wait ${Math.floor(r() * 300)} us`,
  ],
  [
    3,
    (r, db, app) =>
      `LOG C-0x55d4${Math.floor(r() * 0xffffff).toString(16)}: ${db}/${app}@10.0.20.${10 + Math.floor(r() * 200)}:${30000 + Math.floor(r() * 30000)} login attempt: db=${db} user=${app} tls=no`,
  ],
  [
    2,
    (r, db, app) =>
      `LOG C-0x55d4${Math.floor(r() * 0xffffff).toString(16)}: ${db}/${app}@10.0.20.${10 + Math.floor(r() * 200)}:${30000 + Math.floor(r() * 30000)} closing because: client close request (age=${Math.floor(r() * 600)}s)`,
  ],
  [
    1,
    (r, db) => `WARNING C-0x55d4${Math.floor(r() * 0xffffff).toString(16)}: ${db}/reporting@10.0.30.7:${30000 + Math.floor(r() * 30000)} pooler error: no more connections allowed (max_client_conn)`,
  ],
  [
    1,
    (r, db, app) => `LOG C-0x55d4${Math.floor(r() * 0xffffff).toString(16)}: ${db}/${app}@10.0.20.${10 + Math.floor(r() * 200)}:${30000 + Math.floor(r() * 30000)} login failed: db=${db} user=${app}`,
  ],
];

function pgbouncerLogs(r: Rand, now: number): LogEvent[] {
  const total = BOUNCER.reduce((a, [w]) => a + w, 0);
  return Array.from({ length: 240 }, (_, i) => {
    const ts = Math.round(now - r() * 24 * HOUR);
    let n = r() * total;
    const [, body] = BOUNCER.find(([w]) => (n -= w) < 0) ?? BOUNCER[0];
    const c = Math.floor(r() * HOSTS.length);
    const message = `${isoDate(ts)} UTC [1] ${body(r, ['orders', 'catalog', 'billing', 'inventory'][c], APPS[c].replace(/-/g, '_'))}`;
    return parsed('pgbouncer', { ts, i, host: HOSTS[c], service: 'pgbouncer', source: 'pgbouncer', message, tags: { env: c === 3 ? 'qa' : 'prod' } });
  });
}

/** Redis's log: `pid:role date level message`, the level a symbol (. debug, - verbose, * notice, # warning). */
const REDIS: [weight: number, line: (r: Rand) => string][] = [
  [4, (r) => `* ${Math.floor(1 + r() * 100)} changes in 300 seconds. Saving...`],
  [4, (r) => `* Background saving started by pid ${1000 + Math.floor(r() * 9000)}`],
  [4, () => '* Background saving terminated with success'],
  [2, () => '* DB saved on disk'],
  [1, () => '# WARNING: The TCP backlog setting of 511 cannot be enforced because /proc/sys/net/core/somaxconn is set to the lower value of 128.'],
  [1, () => '# Client id=4182 addr=10.0.14.71:51022 laddr=10.0.12.5:6379 fd=12 name= age=0 idle=0 flags=N db=0 closed for overcoming of output buffer limits.'],
  [1, () => '# Error accepting a client connection: error:0A000126:SSL routines::unexpected eof while reading'],
  [1, (r) => `- DB 0: ${Math.floor(1000 + r() * 90000)} keys (${Math.floor(r() * 900)} volatile) in 131072 slots HT.`],
];

function redisLogs(r: Rand, now: number): LogEvent[] {
  const total = REDIS.reduce((a, [w]) => a + w, 0);
  return Array.from({ length: 300 }, (_, i) => {
    const ts = Math.round(now - r() * 24 * HOUR);
    let n = r() * total;
    const [, body] = REDIS.find(([w]) => (n -= w) < 0) ?? REDIS[0];
    const c = Math.floor(r() * HOSTS.length);
    const d = new Date(ts);
    const date = `${pad(d.getUTCDate())} ${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())}.${pad(d.getUTCMilliseconds(), 3)}`;
    const message = `1:${c === 2 ? 'S' : 'M'} ${date} ${body(r)}`;
    return parsed('redisdb', { ts, i, host: HOSTS[c], service: 'redis', source: 'redis', message, tags: { env: c === 3 ? 'qa' : 'prod' } });
  });
}

/** ClickHouse's server log: `date [ thread ] {query id} <Level> component: message`. */
const CLICKHOUSE: [weight: number, level: string, line: (r: Rand) => string][] = [
  [
    5,
    'Debug',
    (r) =>
      `executeQuery: (from 10.0.40.${10 + Math.floor(r() * 200)}:${30000 + Math.floor(r() * 30000)}) SELECT toStartOfHour(timestamp) AS hour, count() FROM events.page_views WHERE timestamp > now() - INTERVAL 1 DAY GROUP BY hour`,
  ],
  [3, 'Information', (r) => `executeQuery: Read ${Math.floor(r() * 9e6)} rows, ${(r() * 400).toFixed(2)} MiB in ${(r() * 2).toFixed(6)} sec.`],
  [3, 'Trace', () => 'events.page_views (ReplicatedMergeTreeQueue): Loaded queue'],
  [2, 'Debug', (r) => `events.page_views (MergeTreeBackgroundExecutor): Merged ${2 + Math.floor(r() * 8)} parts`],
  [1, 'Warning', () => 'events.page_views (ReplicatedMergeTreeRestartingThread): Table is in readonly mode'],
  [1, 'Error', () => 'executeQuery: Code: 241. DB::Exception: Memory limit (for query) exceeded: would use 9.31 GiB. (MEMORY_LIMIT_EXCEEDED)'],
];

function clickhouseLogs(r: Rand, now: number): LogEvent[] {
  const total = CLICKHOUSE.reduce((a, [w]) => a + w, 0);
  return Array.from({ length: 240 }, (_, i) => {
    const ts = Math.round(now - r() * 24 * HOUR);
    let n = r() * total;
    const [, level, body] = CLICKHOUSE.find(([w]) => (n -= w) < 0) ?? CLICKHOUSE[0];
    const c = Math.floor(r() * HOSTS.length);
    const query =
      level === 'Trace'
        ? '{}'
        : `{${[8, 4, 4, 4, 12]
            .map((w) =>
              Math.floor(r() * 16 ** w)
                .toString(16)
                .padStart(w, '0'),
            )
            .join('-')}}`;
    const message = `${isoDate(ts, '.')}${pad(Math.floor(r() * 1000), 3)} [ ${10 + Math.floor(r() * 300)} ] ${query} <${level}> ${body(r)}`;
    return parsed('clickhouse', { ts, i, host: HOSTS[c], service: 'clickhouse', source: 'clickhouse', message, tags: { env: c === 3 ? 'qa' : 'prod' } });
  });
}

/** The four datastores' logs over the last day. */
export function datastoreLogs(r: Rand, now: number): LogEvent[] {
  return [...postgresLogs(r, now), ...pgbouncerLogs(r, now), ...redisLogs(r, now), ...clickhouseLogs(r, now)];
}
