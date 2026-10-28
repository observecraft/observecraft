import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
});
const apm = await import('../web/src/api/mock/apm.ts');

const MIN = 60_000;
const now = Date.now();
const lastHour = { from: now - 60 * MIN, to: now };
const lastWeek = { from: now - 7 * 24 * 60 * MIN, to: now };

const isTree = (trace) => {
  const ids = new Set(trace.spans.map((s) => s.spanId));
  assert.equal(ids.size, trace.spans.length, 'span ids are unique');
  const roots = trace.spans.filter((s) => s.parentSpanId === '');
  assert.equal(roots.length, 1, 'one root span');
  for (const s of trace.spans) {
    assert.match(s.spanId, /^[0-9a-f]{16}$/);
    if (s !== roots[0]) assert.ok(ids.has(s.parentSpanId), `parent of ${s.name} is in the trace`);
    assert.ok(s.startUs >= 0 && s.durationUs > 0);
  }
  const byId = new Map(trace.spans.map((s) => [s.spanId, s]));
  for (const s of trace.spans) if (s.parentSpanId) assert.ok(s.startUs >= byId.get(s.parentSpanId).startUs, 'children start after their parent');
  return roots[0];
};

test('trace search rows open complete traces whose root is the gateway', () => {
  const page = apm.searchSpans({ range: lastHour, query: 'service:api-gateway', limit: 20 });
  assert.equal(page.spans.length, 20);
  assert.ok(page.total >= 20);
  for (const row of page.spans) {
    assert.match(row.traceId, /^[0-9a-f]{32}$/);
    assert.equal(row.service, 'api-gateway');
    assert.equal(row.kind, 'Server');
    const trace = apm.getTrace(row.traceId);
    assert.equal(trace.traceId, row.traceId);
    const root = isTree(trace);
    assert.equal(root.service, 'api-gateway');
    assert.equal(root.spanId, row.spanId);
    assert.ok(
      trace.spans.some((s) => s.service !== 'api-gateway'),
      'the gateway calls downstream services',
    );
    assert.ok(
      trace.spans.some((s) => s.attributes['db.system'] || s.attributes['messaging.system']),
      'a database, cache or queue span',
    );
  }
  const sorted = page.spans.map((s) => s.timestamp);
  assert.deepEqual(
    sorted,
    [...sorted].sort((a, b) => b - a),
    'newest first',
  );
  assert.throws(() => apm.getTrace('ffffffffffffffffffffffffffffffff'), /not found/);
});

test('trace search paginates with a cursor without repeating rows', () => {
  const first = apm.searchSpans({ range: lastHour, query: '', limit: 50 });
  assert.ok(first.nextCursor);
  const second = apm.searchSpans({ range: lastHour, query: '', limit: 50, cursor: first.nextCursor });
  assert.equal(second.spans.length, 50);
  const seen = new Set(first.spans.map((s) => s.spanId));
  for (const s of second.spans) assert.ok(!seen.has(s.spanId));
  assert.ok(second.spans[0].timestamp <= first.spans.at(-1).timestamp);
  assert.deepEqual(apm.searchSpans({ range: lastHour, query: '', limit: 50 }), first, 'deterministic');
});

test('trace logs carry the trace id and the spans that wrote them', () => {
  const row = apm.searchSpans({ range: lastHour, query: 'service:api-gateway', limit: 1 }).spans[0];
  const trace = apm.getTrace(row.traceId);
  const logs = apm.traceLogs({ from: row.timestamp - 3_600_000, to: row.timestamp + 3_600_000 }, row.traceId);
  assert.ok(logs.logs.length > 0);
  const spanIds = new Set(trace.spans.map((s) => s.spanId));
  for (const l of logs.logs) {
    assert.equal(l.traceId, row.traceId);
    assert.ok(spanIds.has(l.spanId));
  }
  const service = apm.serviceLogs(lastHour, 'billing');
  assert.ok(service.logs.length > 0 && service.logs.every((l) => l.service === 'billing'));
});

test('dependencies are exactly the calls found in the spans', () => {
  const traces = apm.searchSpans({ range: lastHour, query: 'service:api-gateway', limit: 300 }).spans.map((r) => apm.getTrace(r.traceId));
  const calls = new Set();
  for (const t of traces) for (const s of t.spans) if (s.attributes['peer.service']) calls.add(`${s.service}>${s.attributes['peer.service']}`);
  const deps = apm.getDependencies({ range: lastHour, service: 'order-service' });
  const downstream = deps.downstream.map((d) => d.service).sort();
  const upstream = deps.upstream.map((d) => d.service).sort();
  assert.deepEqual(
    downstream,
    [...calls]
      .filter((c) => c.startsWith('order-service>'))
      .map((c) => c.split('>')[1])
      .sort(),
  );
  assert.deepEqual(
    upstream,
    [...calls]
      .filter((c) => c.endsWith('>order-service'))
      .map((c) => c.split('>')[0])
      .sort(),
  );
  assert.equal(deps.language, 'go');
  for (const d of deps.downstream) {
    assert.equal(d.errorSeries.length, 20);
    assert.equal(d.p95Series.length, 20);
  }
  const postgres = deps.downstream.find((d) => d.service === 'postgres');
  assert.equal(postgres.kind, 'db');
  assert.equal(postgres.technology, 'postgresql');
});

test('services list instrumented and inferred services; env narrows it', () => {
  const all = apm.listServices({ range: lastHour });
  const names = all.map((s) => s.service);
  for (const s of ['api-gateway', 'storefront-api', 'order-service', 'billing', 'catalog-service', 'inventory-service', 'postgres', 'redis']) assert.ok(names.includes(s), s);
  assert.equal(all.find((s) => s.service === 'postgres').instrumented, false);
  assert.equal(all.find((s) => s.service === 'api-gateway').kind, 'web');
  assert.deepEqual(apm.listEnvs(lastHour), ['prod', 'staging']);
  const staging = apm.listServices({ range: lastHour, env: 'staging' });
  const gw = (list) => list.find((s) => s.service === 'api-gateway').requests;
  assert.ok(gw(staging) > 0 && gw(staging) < gw(all) / 3);
  for (const s of all) assert.ok(s.p50Ms <= s.p95Ms && s.p95Ms <= s.p99Ms, s.service);
});

test('error groups point at sample traces that contain that error', () => {
  const groups = apm.listSpanErrors({ range: lastHour });
  assert.ok(groups.length > 0);
  for (const g of groups) {
    const trace = apm.getTrace(g.sampleTraceId);
    const entry = (s) => s.kind === 'Server' || s.kind === 'Consumer';
    const span = trace.spans.find((s) => entry(s) && s.service === g.service && s.name === g.resource && s.status === 'Error');
    assert.ok(span, `${g.service} ${g.resource}`);
    assert.equal(span.statusMessage, g.message);
    assert.equal(span.events.find((e) => e.name === 'exception').attributes['exception.type'], g.errorType);
  }
  const billing = apm.listSpanErrors({ range: lastHour, service: 'billing' });
  assert.ok(billing.length > 0 && billing.every((g) => g.service === 'billing'));
});

test('service summary series covers the range and agrees with its totals', () => {
  const s = apm.getServiceSummary({ range: lastHour, service: 'order-service' });
  assert.equal(s.instrumented, true);
  assert.ok(s.series.length >= 59);
  assert.ok(s.series[0].t <= lastHour.from + s.bucketMs && s.series.at(-1).t >= lastHour.to - s.bucketMs);
  assert.equal(
    s.requests,
    s.series.reduce((a, b) => a + b.hits, 0),
  );
  assert.ok(s.resources.includes('POST /orders'));
  assert.deepEqual(s.hosts, ['i-0a1b2c3d4e5f60001']);
  const one = apm.getServiceSummary({ range: lastHour, service: 'order-service', resource: 'POST /orders' });
  assert.ok(one.requests > 0 && one.requests < s.requests);
  const week = apm.getServiceSummary({ range: lastWeek, service: 'order-service' });
  assert.ok(week.series[0].t <= lastWeek.from + week.bucketMs && week.series.at(-1).t >= lastWeek.to - week.bucketMs);
  assert.ok(week.requests > s.requests * 50);
  assert.ok(apm.listDeployments({ range: lastWeek, service: 'order-service' }).length >= 2);
});

test('span explorer filters, facets and pagination', () => {
  const q = { range: lastHour, query: '', filters: { service: ['billing'] } };
  const page = apm.exploreSpans(q, 'date', 0, 50);
  assert.equal(page.spans.length, 50);
  assert.ok(page.spans.every((s) => s.service === 'billing'));
  const next = apm.exploreSpans(q, 'date', 50, 50);
  const ids = new Set(page.spans.map((s) => s.spanId));
  assert.ok(next.spans.every((s) => !ids.has(s.spanId)));
  assert.equal(page.total, next.total);
  const all = apm.exploreSpans({ range: lastHour }, 'date', 0, 10);
  assert.ok(page.total < all.total);
  const byQuery = apm.exploreSpans({ range: lastHour, query: 'service:billing' }, 'date', 0, 10);
  assert.equal(byQuery.total, page.total);
  const slow = apm.exploreSpans(q, 'duration', 0, 20).spans.map((s) => s.durationMs);
  assert.deepEqual(
    slow,
    [...slow].sort((a, b) => b - a),
  );

  const facets = apm.spanFacets(q, ['service', 'status']);
  // A facet ignores its own selection, so other services stay visible.
  assert.ok(facets.facets.service.length > 3);
  assert.equal(
    facets.facets.status.reduce((a, v) => a + v.count, 0),
    page.total,
  );
  assert.throws(() => apm.spanFacets(q, ['nope']), /unknown dimension/);

  const traces = apm.exploreSpans({ range: lastHour, scope: 'traces' }, 'date', 0, 10);
  for (const s of traces.spans) assert.ok(traces.traces[s.traceId].spans === apm.getTrace(s.traceId).spans.length);
});

test('span analytics and summary buckets cover the range', () => {
  const q = { range: lastHour, scope: 'service_entry' };
  const a = apm.spanAnalytics(q, 'service', 'count', 3);
  assert.equal(a.series.length, 3);
  assert.equal(a.times.length, Math.floor(lastHour.to / a.stepMs) - Math.floor(lastHour.from / a.stepMs) + 1);
  for (const s of a.series) {
    assert.equal(s.values.length, a.times.length);
    assert.equal(
      s.values.reduce((x, v) => x + v, 0),
      s.total,
    );
  }
  const sum = apm.spanSummary(q);
  assert.equal(sum.hits.length, sum.times.length);
  assert.equal(
    sum.hits.reduce((x, v) => x + v, 0),
    apm.exploreSpans(q, 'date', 0, 1).total,
  );
  assert.ok(sum.overallP95Ms > 0);
  const p = apm.spanPercentile('order-service', 'POST /orders', 1e9, now - 5 * MIN);
  assert.equal(p.percentile, 100);
  assert.ok(p.peers > 0);
});

test('recommendations and anomalies come from the generated spans', () => {
  const recs = apm.listRecommendations({ range: lastHour });
  assert.ok(recs.some((r) => r.kind === 'repeated_sequential_calls' && r.service === 'catalog-service'));
  assert.ok(recs.some((r) => r.kind === 'slow_endpoint'));
  const anomalies = apm.listAnomalies({ range: { from: now - 30 * MIN, to: now } });
  assert.ok(anomalies.some((a) => a.service === 'catalog-service' && a.current > a.previous));
});
