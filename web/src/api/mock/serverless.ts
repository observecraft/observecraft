// In-browser Serverless (AWS Lambda) for mock mode: the demo shop's fictional Lambda functions (webhooks,
// mailers, sync jobs, image processing) in account 123456789012, evaluated like the API: totals over the
// range, insights and upgrades from the same thresholds, facets over every function and the query applied
// to the list.
import type {
  LambdaFacet,
  LambdaFunction,
  LambdaInsight,
  LambdaInvocation,
  LambdaSource,
  LambdaUpgrade,
  ServerlessInvocationsRequest,
  ServerlessSearchPage,
  ServerlessSearchRequest,
  ServerlessSeries,
  ServerlessSummary,
  ServerlessTimeseriesRequest,
} from '../serverless/client';
import { parseQuery, type Term } from '../../lib/searchQuery';
import { rng } from './generator';
import { matchesTerms } from './match';

const ACCOUNT = '123456789012';

// ---------------------------------------------------------------- the API's rules, mirrored

/** AWS Lambda list prices (USD): per request and per GB-second by architecture. */
const PRICE_PER_REQUEST = 0.2 / 1_000_000;
const PRICE_GB_S_X86 = 0.0000166667;
const PRICE_GB_S_ARM = 0.0000133334;

/** Estimated cost of `invocations` of `avgMs` (billed per started ms) with `memoryMb` allocated. */
export function estimateCost(invocations: number, avgMs: number, memoryMb: number, architecture?: string | null): number {
  const price = architecture && (architecture.startsWith('arm') || architecture.startsWith('aarch')) ? PRICE_GB_S_ARM : PRICE_GB_S_X86;
  const gbS = invocations * (Math.ceil(Math.max(avgMs, 1)) / 1000) * (memoryMb / 1024);
  return invocations * PRICE_PER_REQUEST + gbS * price;
}

/** The newest runtime of each family AWS Lambda offers (as the API's table). */
const LATEST_RUNTIMES: [string, string][] = [
  ['nodejs', 'nodejs24.x'],
  ['python', 'python3.14'],
  ['java', 'java25'],
  ['dotnet', 'dotnet10'],
  ['ruby', 'ruby3.4'],
  ['provided', 'provided.al2023'],
  ['go', 'provided.al2023'],
];

export function versionParts(v: string): number[] {
  return v
    .split(/[^0-9]+/)
    .filter(Boolean)
    .map(Number);
}

function compareParts(a: number[], b: number[]): number {
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if (i >= a.length) return -1;
    if (i >= b.length) return 1;
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return 0;
}

function runtimeVersion(runtime: string, family: string): number[] {
  const rest = runtime.slice(family.length);
  return versionParts(family === 'provided' ? rest : rest.split('.al')[0]);
}

/** The runtime to move to when `runtime` is not its family's newest. */
export function runtimeUpgrade(runtime: string): string | null {
  const rt = runtime.trim().toLowerCase();
  const hit = LATEST_RUNTIMES.find(([f]) => rt.startsWith(f));
  if (!hit) return null;
  const [family, latest] = hit;
  if (family === 'go') return latest;
  return compareParts(runtimeVersion(rt, family), runtimeVersion(latest, family)) < 0 ? latest : null;
}

export const THRESHOLDS = { highErrorPct: 5, coldStartPct: 10, coldStartMinInvocations: 10, highMemoryPct: 90, lowMemoryPct: 20, overProvisionedMinMb: 512 };

export function detectInsights(f: LambdaFunction): LambdaInsight[] {
  const out: LambdaInsight[] = [];
  if (f.errorRatePct != null && (f.errors ?? 0) >= 1 && f.errorRatePct >= THRESHOLDS.highErrorPct) out.push({ kind: 'high_errors', value: f.errorRatePct });
  if (f.coldStartPct != null && (f.invocations ?? 0) >= THRESHOLDS.coldStartMinInvocations && f.coldStartPct >= THRESHOLDS.coldStartPct) out.push({ kind: 'cold_starts', value: f.coldStartPct });
  for (const [kind, v] of [
    ['timeouts', f.timeouts],
    ['out_of_memory', f.outOfMemory],
    ['throttles', f.throttles],
  ] as const) {
    if (v != null && v >= 1) out.push({ kind, value: v });
  }
  if (f.memoryUsedPct != null) {
    if (f.memoryUsedPct >= THRESHOLDS.highMemoryPct && (f.outOfMemory ?? 0) < 1) out.push({ kind: 'high_memory', value: f.memoryUsedPct });
    else if (f.memoryUsedPct <= THRESHOLDS.lowMemoryPct && (f.memoryMb ?? 0) >= THRESHOLDS.overProvisionedMinMb) out.push({ kind: 'over_provisioned', value: f.memoryUsedPct });
  }
  return out;
}

/** Outdated runtimes, and Datadog libraries / extensions older than the newest the fleet runs. */
export function detectUpgrades(functions: LambdaFunction[]): void {
  const newest = (key: string) =>
    functions
      .map((f) => f.tags[key])
      .filter((v): v is string => !!v)
      .sort((a, b) => compareParts(versionParts(b), versionParts(a)))[0];
  const library = newest('datadog_lambda');
  const extension = newest('dd_extension_version');
  for (const f of functions) {
    const ups: LambdaUpgrade[] = [];
    const rt = f.runtime ? runtimeUpgrade(f.runtime) : null;
    if (f.runtime && rt) ups.push({ component: 'runtime', current: f.runtime, latest: rt });
    for (const [key, component, latest] of [
      ['datadog_lambda', 'datadog_lambda', library],
      ['dd_extension_version', 'extension', extension],
    ] as const) {
      const cur = f.tags[key];
      if (cur && latest && compareParts(versionParts(cur), versionParts(latest)) < 0) ups.push({ component, current: cur, latest });
    }
    f.upgrades = ups;
  }
}

const pct = (part: number | null, whole: number | null) => (part != null && whole ? Math.round((part / whole) * 1000) / 10 : null);

// ---------------------------------------------------------------- the demo functions

interface Seed {
  name: string;
  region: string;
  service: string;
  team: string;
  env: string;
  stage: string;
  runtime: string;
  memoryMb: number;
  architecture: 'x86_64' | 'arm64';
  /** Invocations per minute. */
  rate: number;
  errorPct: number;
  coldPct: number;
  avgMs: number;
  p95Factor: number;
  /** Peak memory used, as a fraction of the allocation. */
  memUsed: number;
  /** Per hour. */
  timeouts?: number;
  oom?: number;
  throttles?: number;
  trigger: string;
  datadogLambda: string;
  extension: string;
  sources: LambdaSource[];
  logs: boolean;
  /** Minutes before the end of the range the current version was deployed, if it was. */
  deployedAgoMin?: number;
}

const ENHANCED: LambdaSource[] = ['enhanced_metrics', 'traces'];

const SEEDS: Seed[] = [
  {
    name: 'checkout-webhook',
    region: 'us-east-1',
    service: 'checkout',
    team: 'payments',
    env: 'prod',
    stage: 'prod',
    runtime: 'nodejs20.x',
    memoryMb: 1024,
    architecture: 'arm64',
    rate: 180,
    errorPct: 6.4,
    coldPct: 2,
    avgMs: 210,
    p95Factor: 3.1,
    memUsed: 0.46,
    trigger: 'api-gateway',
    datadogLambda: 'v10.12.1',
    extension: '88',
    sources: ENHANCED,
    logs: true,
    deployedAgoMin: 42,
  },
  {
    name: 'payment-confirmation',
    region: 'us-east-1',
    service: 'checkout',
    team: 'payments',
    env: 'prod',
    stage: 'prod',
    runtime: 'nodejs22.x',
    memoryMb: 512,
    architecture: 'arm64',
    rate: 95,
    errorPct: 0.4,
    coldPct: 3,
    avgMs: 140,
    p95Factor: 2.2,
    memUsed: 0.52,
    trigger: 'sqs',
    datadogLambda: 'v10.12.1',
    extension: '88',
    sources: ENHANCED,
    logs: true,
  },
  {
    name: 'order-confirmation-mailer',
    region: 'us-east-1',
    service: 'notifications',
    team: 'customer-comms',
    env: 'prod',
    stage: 'prod',
    runtime: 'python3.9',
    memoryMb: 256,
    architecture: 'x86_64',
    rate: 70,
    errorPct: 1.1,
    coldPct: 14,
    avgMs: 380,
    p95Factor: 2.6,
    memUsed: 0.94,
    trigger: 'sns',
    datadogLambda: 'v6.98.0',
    extension: '81',
    sources: ENHANCED,
    logs: true,
  },
  {
    name: 'cart-abandonment-notifier',
    region: 'us-east-1',
    service: 'notifications',
    team: 'customer-comms',
    env: 'prod',
    stage: 'prod',
    runtime: 'python3.13',
    memoryMb: 2048,
    architecture: 'x86_64',
    rate: 6,
    errorPct: 0,
    coldPct: 40,
    avgMs: 95,
    p95Factor: 1.8,
    memUsed: 0.08,
    trigger: 'eventbridge',
    datadogLambda: 'v6.104.0',
    extension: '88',
    sources: ENHANCED,
    logs: true,
  },
  {
    name: 'order-events-consumer',
    region: 'us-east-1',
    service: 'order-service',
    team: 'orders',
    env: 'prod',
    stage: 'prod',
    runtime: 'java21',
    memoryMb: 1536,
    architecture: 'x86_64',
    rate: 240,
    errorPct: 0.2,
    coldPct: 1,
    avgMs: 160,
    p95Factor: 2.4,
    memUsed: 0.61,
    throttles: 18,
    trigger: 'kinesis',
    datadogLambda: '',
    extension: '88',
    sources: ['enhanced_metrics', 'integration_metrics', 'traces'],
    logs: true,
  },
  {
    name: 'inventory-sync',
    region: 'eu-west-1',
    service: 'inventory-service',
    team: 'inventory',
    env: 'prod',
    stage: 'prod',
    runtime: 'python3.12',
    memoryMb: 512,
    architecture: 'arm64',
    rate: 12,
    errorPct: 2.5,
    coldPct: 6,
    avgMs: 14_200,
    p95Factor: 1.25,
    memUsed: 0.72,
    timeouts: 3,
    trigger: 'eventbridge',
    datadogLambda: 'v6.104.0',
    extension: '86',
    sources: ['otel_metrics', 'traces'],
    logs: false,
  },
  {
    name: 'inventory-reservation-expirer',
    region: 'eu-west-1',
    service: 'inventory-service',
    team: 'inventory',
    env: 'prod',
    stage: 'prod',
    runtime: 'go1.x',
    memoryMb: 128,
    architecture: 'x86_64',
    rate: 30,
    errorPct: 0,
    coldPct: 2,
    avgMs: 35,
    p95Factor: 1.6,
    memUsed: 0.4,
    trigger: 'eventbridge',
    datadogLambda: '',
    extension: '',
    sources: ['integration_metrics'],
    logs: false,
  },
  {
    name: 'image-resizer',
    region: 'us-east-1',
    service: 'catalog-service',
    team: 'catalog',
    env: 'prod',
    stage: 'prod',
    runtime: 'nodejs18.x',
    memoryMb: 1024,
    architecture: 'x86_64',
    rate: 55,
    errorPct: 1.6,
    coldPct: 5,
    avgMs: 900,
    p95Factor: 2.1,
    memUsed: 0.99,
    oom: 4,
    trigger: 's3',
    datadogLambda: 'v10.4.0',
    extension: '84',
    sources: ENHANCED,
    logs: true,
  },
  {
    name: 'catalog-search-indexer',
    region: 'us-east-1',
    service: 'catalog-service',
    team: 'catalog',
    env: 'prod',
    stage: 'prod',
    runtime: 'python3.14',
    memoryMb: 3008,
    architecture: 'arm64',
    rate: 20,
    errorPct: 0.5,
    coldPct: 4,
    avgMs: 2_400,
    p95Factor: 1.7,
    memUsed: 0.55,
    trigger: 'dynamodb',
    datadogLambda: 'v6.104.0',
    extension: '88',
    sources: ENHANCED,
    logs: true,
  },
  {
    name: 'storefront-sitemap-builder',
    region: 'us-east-1',
    service: 'storefront-api',
    team: 'storefront',
    env: 'staging',
    stage: 'staging',
    runtime: 'nodejs24.x',
    memoryMb: 512,
    architecture: 'arm64',
    rate: 2,
    errorPct: 0,
    coldPct: 30,
    avgMs: 4_100,
    p95Factor: 1.3,
    memUsed: 0.33,
    trigger: 'eventbridge',
    datadogLambda: 'v10.12.1',
    extension: '88',
    sources: ENHANCED,
    logs: true,
  },
];

const idOf = (s: Seed) => `${s.region}/${s.name}`;
const arnOf = (s: Seed) => `arn:aws:lambda:${s.region}:${ACCOUNT}:function:${s.name}`;

/** One function's totals over `minutes` (ending at `to`), stable for a given range. */
function functionOver(s: Seed, minutes: number, to: number): LambdaFunction {
  const r = rng(Math.imul(Math.floor(to / 60_000), 2654435761) ^ (s.name.length * 7919) ^ s.name.charCodeAt(0));
  r();
  const jitter = (spread: number) => 1 + (r() - 0.5) * 2 * spread;
  const has = (src: LambdaSource) => s.sources.includes(src);
  const invocations = Math.max(0, Math.round(s.rate * minutes * jitter(0.08)));
  const errors = Math.round((invocations * s.errorPct * jitter(0.15)) / 100);
  const coldStarts = Math.round((invocations * s.coldPct * jitter(0.15)) / 100);
  const avgMs = Math.round(s.avgMs * jitter(0.06) * 10) / 10;
  const perHour = (n?: number) => (n ? Math.max(1, Math.round((n * minutes) / 60)) : has('enhanced_metrics') || has('otel_metrics') ? 0 : null);
  const memUsed = has('enhanced_metrics') || has('otel_metrics') ? Math.round(s.memoryMb * Math.min(1, s.memUsed * jitter(0.02))) : null;
  const tags: Record<string, string> = {
    functionname: s.name,
    region: s.region,
    aws_account: ACCOUNT,
    runtime: s.runtime,
    memorysize: String(s.memoryMb),
    env: s.env,
    service: s.service,
    stage: s.stage,
    team: s.team,
    architecture: s.architecture,
    function_arn: arnOf(s),
    version: s.deployedAgoMin ? '43' : '42',
  };
  if (s.datadogLambda) tags.datadog_lambda = s.datadogLambda;
  if (s.extension) tags.dd_extension_version = s.extension;
  const memoryUsedPct = pct(memUsed, s.memoryMb);
  const f: LambdaFunction = {
    id: idOf(s),
    name: s.name,
    region: s.region,
    account: ACCOUNT,
    runtime: s.runtime,
    service: s.service,
    env: s.env,
    version: tags.version,
    architecture: s.architecture,
    arn: arnOf(s),
    memoryMb: s.memoryMb,
    invocations,
    errors,
    errorRatePct: pct(errors, invocations),
    coldStarts: has('traces') || has('enhanced_metrics') || has('otel_metrics') ? coldStarts : null,
    coldStartPct: has('traces') || has('enhanced_metrics') || has('otel_metrics') ? pct(coldStarts, invocations) : null,
    timeouts: perHour(s.timeouts),
    outOfMemory: perHour(s.oom),
    throttles: has('integration_metrics') ? (perHour(s.throttles) ?? 0) : null,
    durationAvgMs: avgMs,
    durationP95Ms: Math.round(avgMs * s.p95Factor * 10) / 10,
    memoryUsedMb: memUsed,
    memoryUsedPct,
    costUsd: Math.round(estimateCost(invocations, avgMs, s.memoryMb, s.architecture) * 1e6) / 1e6,
    lastDeploy: s.deployedAgoMin && s.deployedAgoMin < minutes ? to - s.deployedAgoMin * 60_000 : null,
    insights: [],
    upgrades: [],
    sources: s.sources,
    hasLogs: s.logs,
    tags,
  };
  f.insights = detectInsights(f);
  return f;
}

function allFunctions(range: { from: number; to: number }): LambdaFunction[] {
  const minutes = Math.max(1, (range.to - range.from) / 60_000);
  const fs = SEEDS.map((s) => functionOver(s, minutes, range.to));
  detectUpgrades(fs);
  return fs;
}

// ---------------------------------------------------------------- search

const FACET_KEYS = ['functionname', 'insight', 'region', 'env', 'stage', 'service', 'runtime', 'memorysize', 'team', 'aws_account', 'architecture'];

function facets(functions: LambdaFunction[]): LambdaFacet[] {
  return FACET_KEYS.map((key) => {
    const counts = new Map<string, number>();
    for (const f of functions) {
      const values = key === 'insight' ? [...new Set(f.insights.map((i) => i.kind))] : f.tags[key] !== undefined ? [f.tags[key]] : [];
      for (const v of values) counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    const values = [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value));
    return { key, values };
  }).filter((f) => f.values.length > 0);
}

/** `insight:` terms match any of the function's insights; the rest its tags and, as free text, its name, service and ARN. */
export function functionMatches(f: LambdaFunction, terms: Term[]): boolean {
  const insights = new Set<string>(f.insights.map((i) => i.kind));
  const rest: Term[] = [];
  for (const t of terms) {
    if (t.kind === 'tag' && t.key === 'insight') {
      if (t.values.some((v) => insights.has(v)) === t.negate) return false;
    } else rest.push(t);
  }
  return matchesTerms((k) => f.tags[k], `${f.name} ${f.service ?? ''} ${f.arn ?? ''}`, rest);
}

export function summarize(functions: LambdaFunction[]): ServerlessSummary {
  const total = (pick: (f: LambdaFunction) => number | null | undefined) => functions.reduce((a, f) => a + (pick(f) ?? 0), 0);
  return {
    functions: functions.length,
    invocations: total((f) => f.invocations),
    errors: total((f) => f.errors),
    coldStarts: total((f) => f.coldStarts),
    costUsd: Math.round(total((f) => f.costUsd) * 1e6) / 1e6,
    withInsights: functions.filter((f) => f.insights.length > 0).length,
    upgradable: functions.filter((f) => f.upgrades.length > 0).length,
  };
}

export async function searchFunctions(req: ServerlessSearchRequest): Promise<ServerlessSearchPage> {
  const all = allFunctions(req.range);
  const terms = parseQuery(req.query ?? '');
  const functions = all.filter((f) => functionMatches(f, terms) && (!req.upgradeAvailable || f.upgrades.length > 0));
  return { enabled: true, summary: summarize(functions), functions, facets: facets(all) };
}

/** Per interval: invocations, errors, duration (p95) and cold starts of each charted function (at most 10). */
export async function serverlessTimeseries(req: ServerlessTimeseriesRequest): Promise<ServerlessSeries[]> {
  const step = Math.max(60_000, Math.ceil((req.range.to - req.range.from) / 60 / 60_000) * 60_000);
  const minutes = step / 60_000;
  const out: ServerlessSeries[] = [];
  for (const id of req.functions.slice(0, 10)) {
    const s = SEEDS.find((x) => idOf(x) === id);
    if (!s) continue;
    const series = { invocations: [] as [number, number][], errors: [] as [number, number][], duration: [] as [number, number][], cold_starts: [] as [number, number][] };
    for (let t = Math.ceil(req.range.from / step) * step; t <= req.range.to; t += step) {
      // A daily swing: busier in the (UTC) afternoon.
      const swing = 0.75 + 0.5 * Math.sin(((t / 3_600_000) % 24) * (Math.PI / 12) - Math.PI / 2) ** 2;
      const f = functionOver({ ...s, rate: s.rate * swing }, minutes, t + step);
      series.invocations.push([t, f.invocations ?? 0]);
      series.errors.push([t, f.errors ?? 0]);
      series.duration.push([t, f.durationP95Ms ?? 0]);
      if (f.coldStarts != null) series.cold_starts.push([t, f.coldStarts]);
    }
    for (const [metric, points] of Object.entries(series)) {
      if (points.length) out.push({ metric, function: id, unit: metric === 'duration' ? 'millisecond' : 'count', points });
    }
  }
  return out;
}

export async function functionInvocations(req: ServerlessInvocationsRequest): Promise<LambdaInvocation[]> {
  const s = SEEDS.find((x) => x.name === req.function.toLowerCase());
  if (!s || !s.sources.includes('traces')) return [];
  const r = rng(s.name.length * 104729 + Math.floor(req.range.to / 60_000));
  const gap = Math.max(1_000, 60_000 / s.rate);
  const hex = (n: number) => Array.from({ length: n }, () => Math.floor(r() * 16).toString(16)).join('');
  return Array.from({ length: 25 }, (_, i) => {
    const error = r() * 100 < Math.max(s.errorPct, 0.01) * 3;
    const cold = r() * 100 < s.coldPct;
    const duration = s.avgMs * (0.5 + r() * (s.p95Factor - 0.3)) + (cold ? 450 : 0);
    return {
      timestamp: Math.round(req.range.to - i * gap * (0.6 + r() * 0.8)),
      traceId: hex(32),
      spanId: hex(16),
      durationMs: Math.round(duration * 10) / 10,
      error,
      coldStart: cold,
      requestId: `${hex(8)}-${hex(4)}-${hex(4)}-${hex(4)}-${hex(12)}`,
      trigger: s.trigger,
      message: error ? (s.timeouts ? 'Task timed out after 15.00 seconds' : 'Error: upstream returned 503') : '',
    };
  });
}
