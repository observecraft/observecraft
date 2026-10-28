// In-browser Data Observability for mock mode: the demo shop's databases (orders and catalog on
// PostgreSQL, inventory on MySQL, a Redis cache, a MongoDB session store) and its data jobs (Airflow
// DAGs and Spark applications), evaluated like the API: facets over everything, the query on the list.
import type { DataJob, DbFacet, DbInstance, DbInstanceDetail, DbInstanceRequest, DbSearchPage, DbSearchRequest, JobFacet, JobSearchPage, JobSearchRequest } from '../databases/client';
import { parseQuery } from '../../lib/searchQuery';
import { rng } from './generator';
import { matchesTerms } from './match';

const GIB = 1024 ** 3;

interface DbSeed extends Omit<DbInstance, 'connections' | 'throughput' | 'cacheHitPct' | 'replicationLagSec' | 'sizeBytes' | 'lastSeen'> {
  connections: number;
  throughput: number;
  cacheHitPct: number | null;
  lagSec: number | null;
  sizeBytes: number;
}

const DB_SEEDS: DbSeed[] = [
  {
    id: 'postgres:i-0a1b2c3d4e5f60010:5432',
    engine: 'postgres',
    host: 'i-0a1b2c3d4e5f60010',
    port: '5432',
    version: '16.4',
    env: 'prod',
    cluster: 'shop-prod',
    service: 'orders-db',
    databases: ['billing', 'orders'],
    maxConnections: 400,
    connections: 182,
    throughput: 640,
    cacheHitPct: 99.4,
    lagSec: null,
    sizeBytes: 182 * GIB,
  },
  {
    id: 'postgres:i-0a1b2c3d4e5f60013:5432',
    engine: 'postgres',
    host: 'i-0a1b2c3d4e5f60013',
    port: '5432',
    version: '16.4',
    env: 'prod',
    cluster: 'shop-prod',
    service: 'orders-db-replica',
    databases: ['billing', 'orders'],
    maxConnections: 400,
    connections: 64,
    throughput: 410,
    cacheHitPct: 97.8,
    lagSec: 2.4,
    sizeBytes: 182 * GIB,
  },
  {
    id: 'postgres:i-0a1b2c3d4e5f60014:5432',
    engine: 'postgres',
    host: 'i-0a1b2c3d4e5f60014',
    port: '5432',
    version: '15.8',
    env: 'staging',
    cluster: 'shop-staging',
    service: 'catalog-db',
    databases: ['catalog'],
    maxConnections: 100,
    connections: 91,
    throughput: 85,
    cacheHitPct: 88.5,
    lagSec: null,
    sizeBytes: 24 * GIB,
  },
  {
    id: 'mysql:i-0a1b2c3d4e5f60011:3306',
    engine: 'mysql',
    host: 'i-0a1b2c3d4e5f60011',
    port: '3306',
    version: '8.0.39',
    env: 'prod',
    cluster: 'shop-prod',
    service: 'inventory-db',
    databases: ['inventory'],
    maxConnections: 500,
    connections: 120,
    throughput: 1250,
    cacheHitPct: 99.1,
    lagSec: 0.3,
    sizeBytes: 64 * GIB,
  },
  {
    id: 'redis:i-0a1b2c3d4e5f60012:6379',
    engine: 'redis',
    host: 'i-0a1b2c3d4e5f60012',
    port: '6379',
    version: '7.2.5',
    env: 'prod',
    cluster: 'shop-prod',
    service: 'session-cache',
    databases: [],
    maxConnections: null,
    connections: 310,
    throughput: 8400,
    cacheHitPct: 93.2,
    lagSec: null,
    sizeBytes: 3.2 * GIB,
  },
  {
    id: 'mongodb:i-0a1b2c3d4e5f60015:27017',
    engine: 'mongodb',
    host: 'i-0a1b2c3d4e5f60015',
    port: '27017',
    version: '7.0.12',
    env: 'prod',
    cluster: 'shop-prod',
    service: 'reviews-db',
    databases: ['reviews'],
    maxConnections: null,
    connections: 48,
    throughput: 220,
    cacheHitPct: null,
    lagSec: 0.8,
    sizeBytes: 41 * GIB,
  },
];

/** Busier in the day, quieter at night; ±10 % noise per 30 s slot. */
function wave(base: number, t: number, seed: number): number {
  const r = rng(Math.imul(Math.floor(t / 30_000), 2654435761) ^ seed);
  r();
  const hour = new Date(t).getUTCHours() + new Date(t).getUTCMinutes() / 60;
  return base * (0.85 + 0.15 * Math.sin(((hour - 8) / 24) * 2 * Math.PI)) * (0.9 + r() * 0.2);
}

function instanceAt(s: DbSeed, t: number, i: number): DbInstance {
  const round = (v: number, d = 2) => Math.round(v * 10 ** d) / 10 ** d;
  return {
    id: s.id,
    engine: s.engine,
    host: s.host,
    port: s.port,
    version: s.version,
    env: s.env,
    cluster: s.cluster,
    service: s.service,
    databases: s.databases,
    maxConnections: s.maxConnections,
    connections: Math.round(Math.min(s.maxConnections ?? Infinity, wave(s.connections, t, i * 13 + 1))),
    throughput: round(wave(s.throughput, t, i * 13 + 2)),
    cacheHitPct: s.cacheHitPct == null ? null : round(Math.min(100, s.cacheHitPct + (wave(1, t, i * 13 + 3) - 1) * 4)),
    replicationLagSec: s.lagSec == null ? null : round(wave(s.lagSec, t, i * 13 + 4)),
    sizeBytes: Math.round(s.sizeBytes),
    lastSeen: t,
  };
}

function dbTags(d: DbInstance): Record<string, string> {
  const out: Record<string, string> = { engine: d.engine, host: d.host };
  if (d.env) out.env = d.env;
  if (d.cluster) out.cluster = d.cluster;
  if (d.service) out.service = d.service;
  return out;
}

function facetsOf<T>(items: T[], keys: string[], tags: (item: T) => Record<string, string>): { key: string; values: { value: string; count: number }[] }[] {
  return keys
    .map((key) => {
      const counts = new Map<string, number>();
      for (const item of items) {
        const v = tags(item)[key];
        if (v !== undefined) counts.set(v, (counts.get(v) ?? 0) + 1);
      }
      return { key, values: [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value)) };
    })
    .filter((f) => f.values.length > 0);
}

export async function searchDatabases(req: DbSearchRequest): Promise<DbSearchPage> {
  const all = DB_SEEDS.map((s, i) => instanceAt(s, req.range.to, i));
  const terms = parseQuery(req.query ?? '');
  const instances = all.filter((d) => matchesTerms((k) => dbTags(d)[k], `${d.id} ${d.databases.join(' ')}`, terms));
  const facets: DbFacet[] = facetsOf(all, ['engine', 'env', 'cluster', 'host', 'service'], dbTags);
  return { enabled: true, instances, facets, unconfigured: [{ host: 'i-0a1b2c3d4e5f60016', engine: 'redis', process: 'redis-server' }] };
}

export async function databaseInstance(req: DbInstanceRequest): Promise<DbInstanceDetail> {
  const i = DB_SEEDS.findIndex((s) => s.id === req.id);
  if (i < 0) return { instance: null, series: [] };
  const step = Math.max(30_000, Math.ceil((req.range.to - req.range.from) / 60 / 30_000) * 30_000);
  const points: Record<string, [number, number][]> = { connections: [], throughput: [], cache_hit: [], replication_lag: [], size: [] };
  for (let t = Math.ceil(req.range.from / step) * step; t <= req.range.to; t += step) {
    const d = instanceAt(DB_SEEDS[i], t, i);
    points.connections.push([t, d.connections ?? 0]);
    points.throughput.push([t, d.throughput ?? 0]);
    if (d.cacheHitPct != null) points.cache_hit.push([t, d.cacheHitPct]);
    if (d.replicationLagSec != null) points.replication_lag.push([t, d.replicationLagSec]);
    points.size.push([t, d.sizeBytes ?? 0]);
  }
  const units: Record<string, string> = { connections: 'connection', throughput: 'operation/second', cache_hit: 'percent', replication_lag: 'second', size: 'byte' };
  return { instance: instanceAt(DB_SEEDS[i], req.range.to, i), series: Object.entries(points).map(([name, pts]) => ({ name, points: pts, unit: units[name] })) };
}

// ---------------------------------------------------------------- data jobs

const JOB_SEEDS: Omit<DataJob, 'lastSeen'>[] = [
  {
    id: 'airflow:catalog_feed_sync',
    framework: 'airflow',
    name: 'catalog_feed_sync',
    env: 'prod',
    cluster: 'shop-data',
    host: 'i-0a1b2c3d4e5f60020',
    succeeded: 21,
    failed: 3,
    running: 0,
    avgDurationMs: 312_000,
    lastDurationMs: 95_000,
    failedTasks: 4,
    succeededTasks: 188,
    status: 'failed',
  },
  {
    id: 'airflow:nightly_orders_export',
    framework: 'airflow',
    name: 'nightly_orders_export',
    env: 'prod',
    cluster: 'shop-data',
    host: 'i-0a1b2c3d4e5f60020',
    succeeded: 1,
    failed: 0,
    running: 0,
    avgDurationMs: 1_840_000,
    lastDurationMs: 1_840_000,
    failedTasks: 0,
    succeededTasks: 12,
    status: 'succeeded',
  },
  {
    id: 'airflow:inventory_snapshot',
    framework: 'airflow',
    name: 'inventory_snapshot',
    env: 'prod',
    cluster: 'shop-data',
    host: 'i-0a1b2c3d4e5f60020',
    succeeded: 24,
    failed: 0,
    running: 0,
    avgDurationMs: 64_000,
    lastDurationMs: 61_000,
    failedTasks: 0,
    succeededTasks: 96,
    status: 'succeeded',
  },
  {
    id: 'spark:recommendations-training',
    framework: 'spark',
    name: 'recommendations-training',
    env: 'prod',
    cluster: 'shop-spark',
    host: 'i-0a1b2c3d4e5f60021',
    succeeded: 37,
    failed: 0,
    running: 2,
    avgDurationMs: null,
    lastDurationMs: null,
    failedTasks: 0,
    succeededTasks: 0,
    status: 'running',
  },
  {
    id: 'spark:sales-aggregation',
    framework: 'spark',
    name: 'sales-aggregation',
    env: 'staging',
    cluster: 'shop-spark',
    host: 'i-0a1b2c3d4e5f60022',
    succeeded: 12,
    failed: 1,
    running: 0,
    avgDurationMs: null,
    lastDurationMs: null,
    failedTasks: 7,
    succeededTasks: 0,
    status: 'failed',
  },
];

const jobTags = (j: DataJob): Record<string, string> => {
  const out: Record<string, string> = { framework: j.framework, status: j.status };
  if (j.env) out.env = j.env;
  if (j.cluster) out.cluster = j.cluster;
  if (j.host) out.host = j.host;
  return out;
};

export async function searchDataJobs(req: JobSearchRequest): Promise<JobSearchPage> {
  const all: DataJob[] = JOB_SEEDS.map((j, i) => ({ ...j, lastSeen: req.range.to - (i + 1) * 7 * 60_000 }));
  const terms = parseQuery(req.query ?? '');
  const jobs = all.filter((j) => matchesTerms((k) => jobTags(j)[k], j.name, terms));
  const facets: JobFacet[] = facetsOf(all, ['framework', 'status', 'env', 'cluster', 'host'], jobTags);
  return { enabled: true, jobs, facets };
}
