// In-browser APM for mock mode: the endpoints of routes/apm.rs and routes/spans.rs evaluated over
// the synthetic traces of apmTraces.ts, with the same filters, units (ms, requests per second) and
// grouping. Long ranges read every n-th minute and scale counts by n, like a sampled rollup.
import type { components } from '../generated/schema';
import type { ApmScope, ServiceScope, SpanMeasure, SpanQuery } from '../apm/client';
import type { TimeRange } from '../types';
import { parseQuery, type Term } from '../../lib/searchQuery';
import { CHUNK_MS, SERVICES, deployedAt, findTrace, tracesOfMinute, type GenSpan, type GenTrace } from './apmTraces';
import { matchesTerms } from './match';

type S = components['schemas'];
type ServiceKind = S['ServiceKind'];

/** Minutes read per query at most; longer ranges are sampled. */
const MAX_CHUNKS = 250;
const DAY = 86_400_000;

interface Scan {
  spans: GenSpan[];
  /** Requests each scanned span stands for. */
  weight: number;
}

function scan(range: TimeRange): Scan {
  const first = Math.floor(range.from / CHUNK_MS) - 1; // traces of the previous minute can reach into the range
  const last = Math.floor(Math.min(range.to, Date.now()) / CHUNK_MS);
  const stride = Math.max(1, Math.ceil((last - first + 1) / MAX_CHUNKS));
  const spans: GenSpan[] = [];
  for (let m = Math.ceil(first / stride) * stride; m <= last; m += stride) {
    for (const t of tracesOfMinute(m)) for (const s of t.spans) if (s.start >= range.from && s.start <= range.to) spans.push(s);
  }
  return { spans, weight: stride };
}

const isEntry = (s: GenSpan) => s.kind === 'Server' || s.kind === 'Consumer';
const isOutbound = (s: GenSpan) => s.kind === 'Client' || s.kind === 'Producer';
const envMatches = (env: string | null | undefined) => (s: GenSpan) => !env || env === '*' || s.trace.env === env;
const secondsOf = (range: TimeRange) => Math.max(1, (range.to - range.from) / 1000);
const rate = (errors: number, hits: number) => (hits ? errors / hits : 0);
const round = (v: number) => (Number.isFinite(v) ? Math.round(v * 1000) / 1000 : 0);

const resourceCache = new Map<string, Record<string, string>>();

/** The resource attributes the span's service reported. */
function resourceOf(s: GenSpan): Record<string, string> {
  const key = `${s.service}|${s.version}|${s.trace.env}`;
  let res = resourceCache.get(key);
  if (!res) {
    const svc = SERVICES[s.service];
    res = {
      'service.name': s.service,
      'service.version': s.version,
      'deployment.environment': s.trace.env,
      'telemetry.sdk.language': svc.lang,
      'host.name': svc.host,
      'k8s.namespace.name': svc.namespace,
    };
    resourceCache.set(key, res);
  }
  return res;
}

/** A search key resolved against a span, as `oc_query::SpansSchema` maps it. */
function field(s: GenSpan, key: string): string | undefined {
  switch (key) {
    case 'service':
      return s.service;
    case 'resource':
    case 'operation':
    case 'name':
      return s.name;
    case 'status':
      return s.error ? 'error' : 'ok';
    case 'kind':
      return s.kind.toLowerCase();
    case 'trace_id':
      return s.trace.traceId;
    case 'env':
      return s.trace.env;
    case 'version':
      return s.version;
    case 'host':
      return SERVICES[s.service].host;
  }
  if (key.startsWith('@')) return s.attrs[key.slice(1)];
  return resourceOf(s)[key];
}

const matchSpan = (s: GenSpan, terms: Term[]) => matchesTerms((k) => field(s, k), `${s.name} ${s.error?.message ?? ''}`, terms);

function quantile(sorted: number[], q: number): number {
  if (!sorted.length) return 0;
  const i = (sorted.length - 1) * q;
  const lo = Math.floor(i);
  return sorted[lo] + (sorted[Math.min(lo + 1, sorted.length - 1)] - sorted[lo]) * (i - lo);
}

interface Agg {
  spans: GenSpan[];
  errors: number;
  totalMs: number;
}

function group(spans: GenSpan[], key: (s: GenSpan) => string): Map<string, Agg> {
  const out = new Map<string, Agg>();
  for (const s of spans) {
    const k = key(s);
    let a = out.get(k);
    if (!a) out.set(k, (a = { spans: [], errors: 0, totalMs: 0 }));
    a.spans.push(s);
    a.errors += s.error ? 1 : 0;
    a.totalMs += s.durationMs;
  }
  return out;
}

function latencies(spans: GenSpan[]) {
  const d = spans.map((s) => s.durationMs).sort((a, b) => a - b);
  return { p50Ms: quantile(d, 0.5), p95Ms: quantile(d, 0.95), p99Ms: quantile(d, 0.99) };
}

function inferredKind(dbSystem: string | undefined, messaging: string | undefined): ServiceKind {
  if (dbSystem === 'redis' || dbSystem === 'memcached') return 'cache';
  if (dbSystem) return 'db';
  if (messaging) return 'queue';
  return 'http';
}

const instrumented = (spans: GenSpan[]) => new Set(spans.filter(isEntry).map((s) => s.service));

/** A service's own entry spans, or callers' spans to it when it is inferred. */
function ofService(service: string, isInstrumented: boolean) {
  return isInstrumented ? (s: GenSpan) => s.service === service && isEntry(s) : (s: GenSpan) => isOutbound(s) && s.attrs['peer.service'] === service;
}

const serviceOf = (isInstrumented: boolean) => (isInstrumented ? (s: GenSpan) => s.service : (s: GenSpan) => s.attrs['peer.service']);

// ================================================================ services

export function listServices(s: ApmScope): S['ServiceStats'][] {
  const { spans, weight } = scan(s.range);
  const scoped = spans.filter(envMatches(s.env));
  const secs = secondsOf(s.range);
  const entries = group(scoped.filter(isEntry), (x) => x.service);
  const out: S['ServiceStats'][] = [...entries].map(([service, a]) => {
    const newest = a.spans.reduce((x, y) => (y.start > x.start ? y : x));
    return {
      service,
      kind: a.spans.some((x) => x.attrs['http.route']) ? 'web' : 'worker',
      language: SERVICES[service].lang,
      version: newest.version,
      lastDeploy: deployedAt(service, Date.now() - 7 * DAY) ?? Date.now() - 7 * DAY,
      requests: a.spans.length * weight,
      reqPerSec: (a.spans.length * weight) / secs,
      errors: a.errors * weight,
      errorRate: rate(a.errors, a.spans.length),
      ...latencies(a.spans),
      instrumented: true,
    };
  });
  const peers = group(
    scoped.filter((x) => isOutbound(x) && x.attrs['peer.service'] && !entries.has(x.attrs['peer.service'])),
    (x) => x.attrs['peer.service'],
  );
  for (const [service, a] of peers) {
    out.push({
      service,
      kind: inferredKind(a.spans[0].attrs['db.system'], a.spans[0].attrs['messaging.system']),
      language: null,
      version: null,
      lastDeploy: null,
      requests: a.spans.length * weight,
      reqPerSec: (a.spans.length * weight) / secs,
      errors: a.errors * weight,
      errorRate: rate(a.errors, a.spans.length),
      ...latencies(a.spans),
      instrumented: false,
    });
  }
  return out.sort((a, b) => b.requests - a.requests);
}

export function listEnvs(range: TimeRange): string[] {
  return [...new Set(scan(range).spans.map((s) => s.trace.env))].sort();
}

// ================================================================ resources

function resourceStats(spans: GenSpan[], weight: number, range: TimeRange, serviceKey: (s: GenSpan) => string): S['ResourceStats'][] {
  const groups = group(spans, (s) => `${serviceKey(s)}\u0000${s.name}`);
  const totals = new Map<string, number>();
  for (const [k, a] of groups) totals.set(k.split('\u0000')[0], (totals.get(k.split('\u0000')[0]) ?? 0) + a.totalMs);
  const secs = secondsOf(range);
  return [...groups]
    .map(([k, a]) => {
      const [service, resource] = k.split('\u0000');
      return {
        service,
        resource,
        requests: a.spans.length * weight,
        reqPerSec: (a.spans.length * weight) / secs,
        errors: a.errors * weight,
        errorRate: rate(a.errors, a.spans.length),
        ...latencies(a.spans),
        timeShare: a.totalMs / Math.max(totals.get(service) ?? 1, 1e-9),
      };
    })
    .sort((a, b) => b.requests - a.requests)
    .slice(0, 500);
}

export function listResources(s: ApmScope & { service?: string | null }): S['ResourceStats'][] {
  const { spans, weight } = scan(s.range);
  const scoped = spans.filter(envMatches(s.env));
  if (!s.service) return resourceStats(scoped.filter(isEntry), weight, s.range, (x) => x.service);
  const inst = instrumented(spans).has(s.service);
  return resourceStats(scoped.filter(ofService(s.service, inst)), weight, s.range, serviceOf(inst));
}

// ================================================================ service summary

function bucketMs(range: TimeRange, target: number): number {
  const raw = (range.to - range.from) / target;
  return [10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000].find((v) => v >= raw) ?? 43_200_000;
}

/** Bucket start times covering the range. */
function bucketTimes(range: TimeRange, step: number): number[] {
  const out: number[] = [];
  for (let t = Math.floor(range.from / step) * step; t <= range.to; t += step) out.push(t);
  return out;
}

function narrowed(s: ServiceScope, spans: GenSpan[], inst: boolean, opts: { resource: boolean; version: boolean; env: boolean }): GenSpan[] {
  const svc = ofService(s.service, inst);
  return spans.filter(
    (x) => svc(x) && (!opts.env || envMatches(s.env)(x)) && (!opts.resource || !s.resource || x.name === s.resource) && (!opts.version || !s.version || s.version === '*' || x.version === s.version),
  );
}

export function getServiceSummary(s: ServiceScope): S['ServiceSummary'] {
  const { spans, weight } = scan(s.range);
  const inst = instrumented(spans).has(s.service);
  const matched = narrowed(s, spans, inst, { resource: true, version: true, env: true });
  const size = bucketMs(s.range, 60);
  const times = bucketTimes(s.range, size);
  const buckets = times.map(() => [] as GenSpan[]);
  for (const x of matched) buckets[Math.floor((x.start - times[0]) / size)]?.push(x);
  const series = times.map((t, i) => {
    const b = buckets[i];
    return { t, hits: b.length * weight, errors: b.filter((x) => x.error).length * weight, ...latencies(b) };
  });
  const requests = series.reduce((a, b) => a + b.hits, 0);
  const errors = series.reduce((a, b) => a + b.errors, 0);
  const errorRate = rate(errors, requests);
  // The header selects list every option, whatever the resource, version and env narrowing.
  const meta = narrowed(s, spans, inst, { resource: false, version: false, env: false });
  const uniq = (f: (x: GenSpan) => string) => [...new Set(meta.map(f))].filter(Boolean).sort();
  const first = meta[0];
  return {
    service: s.service,
    kind: inst ? (meta.some((x) => x.attrs['http.route']) ? 'web' : 'worker') : inferredKind(first?.attrs['db.system'], first?.attrs['messaging.system']),
    instrumented: inst,
    language: inst ? SERVICES[s.service].lang : null,
    health: errorRate > 0.05 ? 'critical' : errorRate > 0.01 ? 'warn' : 'ok',
    requests,
    errors,
    errorRate,
    reqPerSec: requests / secondsOf(s.range),
    p95Ms: Math.max(0, ...series.filter((b) => b.hits > 0).map((b) => b.p95Ms)),
    envs: uniq((x) => x.trace.env),
    versions: uniq((x) => x.version),
    resources: uniq((x) => x.name),
    hosts: uniq((x) => SERVICES[x.service].host),
    series,
    bucketMs: size,
  };
}

export function listDeployments(s: ServiceScope): S['Deployment'][] {
  const { spans, weight } = scan(s.range);
  return [...group(narrowed(s, spans, true, { resource: true, version: false, env: true }), (x) => x.version)]
    .map(([version, a]) => ({
      version,
      firstSeen: Math.floor(Math.min(...a.spans.map((x) => x.start))),
      lastSeen: Math.floor(Math.max(...a.spans.map((x) => x.start))),
      requests: a.spans.length * weight,
      errors: a.errors * weight,
      errorRate: rate(a.errors, a.spans.length),
      p95Ms: latencies(a.spans).p95Ms,
    }))
    .sort((a, b) => b.firstSeen - a.firstSeen);
}

// ================================================================ dependencies & databases

function edges(range: TimeRange, calls: GenSpan[], weight: number, peerOf: (s: GenSpan) => string, known: Set<string>): S['Dependency'][] {
  const BUCKETS = 20;
  const size = Math.max(1, Math.floor((range.to - range.from) / BUCKETS));
  return [...group(calls, peerOf)]
    .map(([peer, a]): S['Dependency'] => {
      const buckets = Array.from({ length: BUCKETS }, () => [] as GenSpan[]);
      for (const x of a.spans) buckets[Math.floor((x.start - range.from) / size)]?.push(x);
      const isKnown = known.has(peer);
      const tech = a.spans[0].attrs['db.system'] || a.spans[0].attrs['messaging.system'];
      return {
        service: peer,
        kind: isKnown ? 'web' : inferredKind(a.spans[0].attrs['db.system'], a.spans[0].attrs['messaging.system']),
        language: isKnown ? SERVICES[peer].lang : null,
        technology: isKnown ? null : (tech ?? null),
        requests: a.spans.length * weight,
        reqPerSec: (a.spans.length * weight) / secondsOf(range),
        errors: a.errors * weight,
        errorRate: rate(a.errors, a.spans.length),
        p95Ms: Math.max(0, ...buckets.filter((b) => b.length).map((b) => latencies(b).p95Ms)),
        errorSeries: buckets.map((b) => b.filter((x) => x.error).length * weight),
        p95Series: buckets.map((b) => latencies(b).p95Ms),
      };
    })
    .sort((a, b) => b.requests - a.requests);
}

export function getDependencies(s: ServiceScope): S['Dependencies'] {
  const { spans, weight } = scan(s.range);
  const known = instrumented(spans);
  const calls = spans.filter((x) => envMatches(s.env)(x) && isOutbound(x) && x.attrs['peer.service']);
  return {
    downstream: edges(
      s.range,
      calls.filter((x) => x.service === s.service),
      weight,
      (x) => x.attrs['peer.service'],
      known,
    ),
    upstream: edges(
      s.range,
      calls.filter((x) => x.attrs['peer.service'] === s.service),
      weight,
      (x) => x.service,
      known,
    ),
    language: known.has(s.service) ? SERVICES[s.service].lang : null,
  };
}

export function listDatabases(s: ServiceScope): S['DatabaseStats'][] {
  const { spans, weight } = scan(s.range);
  const calls = spans.filter((x) => envMatches(s.env)(x) && x.kind === 'Client' && x.service === s.service && x.attrs['db.system']);
  const groups = group(calls, (x) => [x.attrs['db.system'], x.attrs['db.name'], x.attrs['db.statement']].join('\u0000'));
  const total = Math.max(
    [...groups.values()].reduce((a, g) => a + g.totalMs, 0),
    1e-9,
  );
  return [...groups]
    .map(([k, a]) => {
      const [dbSystem, dbName, statement] = k.split('\u0000');
      return { dbSystem, dbName, statement, requests: a.spans.length * weight, errors: a.errors * weight, p95Ms: latencies(a.spans).p95Ms, timeShare: a.totalMs / total, sort: a.totalMs };
    })
    .sort((a, b) => b.sort - a.sort)
    .slice(0, 100)
    .map(({ sort: _, ...row }) => row);
}

// ================================================================ errors

export function listSpanErrors(s: ApmScope & { service?: string | null }): S['SpanErrorGroup'][] {
  const { spans, weight } = scan(s.range);
  const failed = spans.filter((x) => x.error && isEntry(x) && envMatches(s.env)(x) && (!s.service || x.service === s.service));
  return [...group(failed, (x) => [x.service, x.name, x.error!.type, x.error!.message].join('\u0000'))]
    .map(([k, a]) => {
      const [service, resource, errorType, message] = k.split('\u0000');
      const starts = a.spans.map((x) => x.start);
      const newest = a.spans.reduce((x, y) => (y.start > x.start ? y : x));
      return {
        service,
        resource,
        errorType,
        message,
        count: a.spans.length * weight,
        firstSeen: Math.floor(Math.min(...starts)),
        lastSeen: Math.floor(Math.max(...starts)),
        sampleTraceId: newest.trace.traceId,
      };
    })
    .sort((a, b) => b.count - a.count)
    .slice(0, 100);
}

// ================================================================ traces

const newestFirst = (a: GenSpan, b: GenSpan) => b.start - a.start || (b.spanId < a.spanId ? -1 : b.spanId > a.spanId ? 1 : 0);
const status = (s: GenSpan) => s.attrs['http.response.status_code'] ?? '';

export function searchSpans(req: ApmScope & { query: string; cursor?: string | null; limit?: number }): S['TraceSearchPage'] {
  const limit = Math.min(500, Math.max(1, req.limit ?? 100));
  const terms = parseQuery(req.query);
  const { spans, weight } = scan(req.range);
  const matched = spans.filter((s) => isEntry(s) && envMatches(req.env)(s) && matchSpan(s, terms)).sort(newestFirst);
  let from = 0;
  if (req.cursor) {
    const [ts, id] = req.cursor.split(':');
    const at = Number(ts);
    if (!id || !Number.isFinite(at)) throw new Error('invalid cursor');
    from = matched.findIndex((s) => s.start < at || (s.start === at && s.spanId < id));
    if (from < 0) from = matched.length;
  }
  const page = matched.slice(from, from + limit);
  const last = page.at(-1);
  return {
    spans: page.map((s) => ({
      traceId: s.trace.traceId,
      spanId: s.spanId,
      timestamp: Math.floor(s.start),
      service: s.service,
      resource: s.name,
      kind: s.kind,
      durationMs: s.durationMs,
      status: s.error ? 'Error' : 'Ok',
      env: s.trace.env,
      httpStatus: status(s),
    })),
    total: matched.length * weight,
    nextCursor: page.length === limit && last ? `${last.start}:${last.spanId}` : null,
  };
}

export function getTrace(traceId: string): S['TraceDetail'] {
  const trace = findTrace(traceId);
  if (!trace) throw new Error('trace not found');
  const t0 = Math.min(...trace.spans.map((s) => s.start));
  const end = Math.max(...trace.spans.map((s) => s.start + s.durationMs));
  return {
    traceId,
    start: Math.floor(t0),
    durationMs: end - t0,
    spans: [...trace.spans]
      .sort((a, b) => a.start - b.start)
      .map((s) => ({
        spanId: s.spanId,
        parentSpanId: s.parentId,
        service: s.service,
        name: s.name,
        kind: s.kind,
        startUs: (s.start - t0) * 1000,
        durationUs: s.durationMs * 1000,
        status: s.error ? 'Error' : 'Ok',
        statusMessage: s.error?.message ?? '',
        attributes: { ...s.attrs },
        resource: { ...resourceOf(s) },
        events: s.error
          ? [
              {
                name: 'exception',
                offsetUs: (s.start + s.durationMs - t0) * 1000,
                attributes: {
                  'exception.type': s.error.type,
                  'exception.message': s.error.message,
                  'exception.stacktrace': `${s.error.type}: ${s.error.message}\n    at ${s.service}.handler`,
                },
              },
            ]
          : [],
      })),
  };
}

// ================================================================ logs written inside traces

/** One log per request handled and one per error, like the instrumented demo services write. */
function logOf(s: GenSpan): S['LogEvent'] {
  const svc = SERVICES[s.service];
  const ms = s.durationMs;
  return {
    id: `log-${s.spanId}`,
    timestamp: Math.floor(s.start + ms),
    status: s.error ? 'error' : 'info',
    service: s.service,
    host: svc.host,
    source: s.service,
    message: s.error ? `${s.name} failed: ${s.error.type}: ${s.error.message}` : `${s.name} handled in ${ms.toFixed(1)} ms`,
    tags: { env: s.trace.env, service: s.service, version: s.version, host: svc.host, kube_namespace: svc.namespace },
    attributes: { 'code.function': `${s.service}.handler`, duration_ms: round(ms) } as unknown as S['LogEvent']['attributes'],
    traceId: s.trace.traceId,
    spanId: s.spanId,
  };
}

const logged = (s: GenSpan) => isEntry(s) || !!s.error;
const newestLogFirst = (a: GenSpan, b: GenSpan) => b.start + b.durationMs - (a.start + a.durationMs);

export function serviceLogs(range: TimeRange, service: string): S['LogSearchPage'] {
  const { spans, weight } = scan(range);
  const matched = spans.filter((s) => s.service === service && logged(s)).sort(newestLogFirst);
  return { logs: matched.slice(0, 25).map(logOf), total: matched.length * weight, nextCursor: null };
}

export function traceLogs(range: TimeRange, traceId: string): S['LogSearchPage'] {
  const trace: GenTrace | undefined = findTrace(traceId);
  const logs = (trace?.spans ?? [])
    .filter(logged)
    .sort(newestLogFirst)
    .map(logOf)
    .filter((l) => l.timestamp >= range.from && l.timestamp <= range.to);
  return { logs: logs.slice(0, 200), total: logs.length, nextCursor: null };
}

// ================================================================ recommendations & anomalies

function endpointStats(range: TimeRange, env: string | null | undefined) {
  const { spans, weight } = scan(range);
  const groups = group(
    spans.filter((s) => isEntry(s) && envMatches(env)(s)),
    (s) => `${s.service}\u0000${s.name}`,
  );
  return new Map([...groups].map(([k, a]) => [k, { hits: a.spans.length * weight, errors: a.errors * weight, p95: latencies(a.spans).p95Ms }] as const).filter(([, e]) => e.hits >= 20));
}

export function listRecommendations(s: ApmScope): S['Recommendation'][] {
  const { spans, weight } = scan(s.range);
  // N+1: the same client call made five or more times under one parent span.
  const repeats = group(
    spans.filter((x) => x.kind === 'Client' && envMatches(s.env)(x)),
    (x) => [x.trace.traceId, x.parentId, x.service, x.name].join('\u0000'),
  );
  const n1 = new Map<string, { traces: number; calls: number }>();
  for (const [k, a] of repeats) {
    if (a.spans.length < 5) continue;
    const [, , service, resource] = k.split('\u0000');
    const e = n1.get(`${service}\u0000${resource}`) ?? { traces: 0, calls: 0 };
    e.traces += 1;
    e.calls += a.spans.length;
    n1.set(`${service}\u0000${resource}`, e);
  }
  const out: S['Recommendation'][] = [...n1]
    .sort((a, b) => b[1].traces - a[1].traces)
    .slice(0, 10)
    .map(([k, e]) => {
      const [service, resource] = k.split('\u0000');
      const traces = e.traces * weight;
      return {
        id: `n1:${service}:${resource}`,
        kind: 'repeated_sequential_calls',
        priority: traces >= 50 ? 'medium' : 'low',
        service,
        resource,
        occurrences: traces,
        requests: e.calls * weight,
        value: e.calls / e.traces,
      };
    });
  for (const [k, e] of endpointStats(s.range, s.env)) {
    const [service, resource] = k.split('\u0000');
    const er = rate(e.errors, e.hits);
    if (er >= 0.02)
      out.push({ id: `err:${service}:${resource}`, kind: 'high_error_rate', priority: er >= 0.1 ? 'high' : 'medium', service, resource, occurrences: e.errors, requests: e.hits, value: er });
    if (e.p95 >= 250) out.push({ id: `slow:${service}:${resource}`, kind: 'slow_endpoint', priority: 'low', service, resource, occurrences: Math.floor(e.hits / 20), requests: e.hits, value: e.p95 });
  }
  const rank = { high: 0, medium: 1, low: 2 };
  return out.sort((a, b) => rank[a.priority] - rank[b.priority] || b.occurrences - a.occurrences);
}

export function listAnomalies(s: ApmScope): S['Anomaly'][] {
  const span = s.range.to - s.range.from;
  const now = endpointStats(s.range, s.env);
  const before = endpointStats({ from: s.range.from - span, to: s.range.from }, s.env);
  const out: S['Anomaly'][] = [];
  for (const [k, e] of now) {
    const b = before.get(k);
    if (!b) continue;
    const [service, resource] = k.split('\u0000');
    if (b.p95 > 0 && e.p95 > b.p95 * 1.5 && e.p95 - b.p95 > 20) out.push({ service, resource, metric: 'p95_latency_ms', previous: b.p95, current: e.p95, requests: e.hits });
    const [en, eb] = [rate(e.errors, e.hits), rate(b.errors, b.hits)];
    if (en - eb > 0.01) out.push({ service, resource, metric: 'error_rate', previous: eb, current: en, requests: e.hits });
  }
  return out;
}

// ================================================================ span explorer

const DIMENSIONS: Record<string, (s: GenSpan) => string> = {
  service: (s) => s.service,
  resource: (s) => s.name,
  env: (s) => s.trace.env,
  status: (s) => (s.error ? 'error' : 'ok'),
  http_status: status,
  method: (s) => s.attrs['http.request.method'] ?? '',
  kind: (s) => s.kind.toLowerCase(),
  host: (s) => SERVICES[s.service].host,
  db_system: (s) => s.attrs['db.system'] ?? '',
  version: (s) => s.version,
};

function dimension(name: string): (s: GenSpan) => string {
  const d = DIMENSIONS[name];
  if (!d) throw new Error(`unknown dimension '${name}'`);
  return d;
}

/** Spans matching the explorer query; `skip` ignores one facet's own selection. */
function where(q: SpanQuery, spans: GenSpan[], skip?: string, durations = true): GenSpan[] {
  const terms = parseQuery(q.query ?? '');
  const filters = Object.entries(q.filters ?? {})
    .filter(([dim, values]) => values.length && dim !== skip)
    .map(([dim, values]) => [dimension(dim), new Set(values)] as const);
  const min = durations && q.minDurationMs && q.minDurationMs > 0 ? q.minDurationMs : null;
  const max = durations && q.maxDurationMs && q.maxDurationMs > 0 ? q.maxDurationMs : null;
  return spans.filter(
    (s) =>
      (q.scope !== 'service_entry' || isEntry(s)) &&
      (q.scope !== 'traces' || s.parentId === '') &&
      (min === null || s.durationMs >= min) &&
      (max === null || s.durationMs <= max) &&
      filters.every(([get, values]) => values.has(get(s))) &&
      matchSpan(s, terms),
  );
}

function explorerRow(s: GenSpan): S['ExplorerSpan'] {
  return {
    traceId: s.trace.traceId,
    spanId: s.spanId,
    timestamp: Math.floor(s.start),
    service: s.service,
    resource: s.name,
    kind: s.kind.toLowerCase(),
    durationMs: s.durationMs,
    status: s.error ? 'error' : 'ok',
    env: s.trace.env,
    method: s.attrs['http.request.method'] ?? '',
    httpStatus: status(s),
    dbSystem: s.attrs['db.system'] ?? '',
    peerService: s.attrs['peer.service'] ?? '',
    host: SERVICES[s.service].host,
  };
}

export function exploreSpans(q: SpanQuery, sort: string, offset: number, limit: number): S['SpanSearchResult'] {
  const { spans, weight } = scan(q.range);
  const matched = where(q, spans).sort(sort === 'duration' ? (a, b) => b.durationMs - a.durationMs : newestFirst);
  const start = Math.min(10_000, Math.max(0, offset));
  const page = matched.slice(start, start + Math.min(500, Math.max(1, limit)));
  const traces: S['SpanSearchResult']['traces'] = {};
  if (q.scope === 'traces') {
    for (const s of page) {
      const t = s.trace.spans;
      traces[s.trace.traceId] = { spans: t.length, services: new Set(t.map((x) => x.service)).size, errors: t.filter((x) => x.error).length };
    }
  }
  const total = matched.length * weight;
  return { spans: page.map(explorerRow), total, perSecond: Math.round((total / secondsOf(q.range)) * 100) / 100, traces };
}

export function spanFacets(q: SpanQuery, dimensions: string[]): S['SpanFacets'] {
  const { spans, weight } = scan(q.range);
  const facets: S['SpanFacets']['facets'] = {};
  for (const dim of dimensions.length ? dimensions : Object.keys(DIMENSIONS)) {
    const get = dimension(dim);
    const counts = new Map<string, number>();
    for (const s of where(q, spans, dim)) {
      const v = get(s);
      if (v) counts.set(v, (counts.get(v) ?? 0) + weight);
    }
    facets[dim] = [...counts]
      .map(([value, count]) => ({ value, count }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 50);
  }
  const durations = where(q, spans, undefined, false).map((s) => s.durationMs);
  return { facets, minDurationMs: durations.length ? Math.min(...durations) : 0, maxDurationMs: durations.length ? Math.max(...durations) : 0 };
}

function stepFor(range: TimeRange, buckets: number): number {
  const raw = (range.to - range.from) / Math.max(1, buckets);
  return [10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000, 3 * 3_600_000, 6 * 3_600_000, 12 * 3_600_000, DAY].find((v) => v >= raw) ?? DAY;
}

function measureOf(measure: SpanMeasure, spans: GenSpan[], weight: number): number {
  switch (measure) {
    case 'count':
      return spans.length * weight;
    case 'errors':
      return spans.filter((s) => s.error).length * weight;
    case 'avg':
      return spans.reduce((a, s) => a + s.durationMs, 0) / spans.length;
    case 'max':
      return Math.max(...spans.map((s) => s.durationMs));
  }
  const l = latencies(spans);
  return measure === 'p50' ? l.p50Ms : measure === 'p95' ? l.p95Ms : l.p99Ms;
}

/** Spans per bucket of `step` over the range. */
function bucketed(range: TimeRange, step: number, spans: GenSpan[]) {
  const times = bucketTimes(range, step);
  const buckets = times.map(() => [] as GenSpan[]);
  for (const s of spans) buckets[Math.floor((s.start - times[0]) / step)]?.push(s);
  return { times, buckets };
}

export function spanAnalytics(q: SpanQuery, groupBy: string | null, measure: SpanMeasure, top: number): S['SpanAnalytics'] {
  const { spans, weight } = scan(q.range);
  const key = groupBy ? dimension(groupBy) : () => 'all';
  const step = stepFor(q.range, 60);
  const counted = measure === 'count' || measure === 'errors';
  const groups = [...group(where(q, spans), key)].sort((a, b) => b[1].spans.length - a[1].spans.length).slice(0, Math.min(50, Math.max(1, top || 10)));
  let times: number[] = bucketTimes(q.range, step);
  const series = groups.map(([g, a]) => {
    const b = bucketed(q.range, step, a.spans);
    times = b.times;
    return { group: g || 'N/A', total: round(measureOf(measure, a.spans, weight)), values: b.buckets.map((x) => (x.length ? round(measureOf(measure, x, weight)) : counted ? 0 : null)) };
  });
  return { stepMs: step, times, series };
}

export function spanSummary(q: SpanQuery): S['SpanSummary'] {
  const { spans, weight } = scan(q.range);
  const matched = where(q, spans);
  const { times, buckets } = bucketed(q.range, stepFor(q.range, 40), matched);
  const l = buckets.map((b) => (b.length ? latencies(b) : null));
  return {
    stepMs: stepFor(q.range, 40),
    times,
    hits: buckets.map((b) => b.length * weight),
    errors: buckets.map((b) => b.filter((s) => s.error).length * weight),
    p50Ms: l.map((x) => (x ? round(x.p50Ms) : null)),
    p95Ms: l.map((x) => (x ? round(x.p95Ms) : null)),
    p99Ms: l.map((x) => (x ? round(x.p99Ms) : null)),
    overallP95Ms: round(latencies(matched).p95Ms),
  };
}

export function spanPercentile(service: string, resource: string, durationMs: number, at: number): S['SpanPercentile'] {
  const { spans, weight } = scan({ from: at - 3_600_000, to: at + 60_000 });
  const peers = spans.filter((s) => s.service === service && s.name === resource);
  const faster = peers.filter((s) => s.durationMs < durationMs).length;
  return { percentile: peers.length ? Math.round((faster * 100) / peers.length) : 0, peers: peers.length * weight };
}
