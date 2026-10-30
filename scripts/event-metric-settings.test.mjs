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

const checks = await import('../web/src/lib/serviceChecks.ts');
const checksMock = await import('../web/src/api/mock/serviceChecks.ts');
const derived = await import('../web/src/lib/derivedMetrics.ts');
const derivedMock = await import('../web/src/api/mock/derivedMetrics.ts');
const engine = await import('../web/src/api/mock/metricsEngine.ts');
const rules = await import('../web/src/lib/metricRules.ts');
const rulesMock = await import('../web/src/api/mock/metricRules.ts');
const correlation = await import('../web/src/lib/eventCorrelation.ts');
const eventSettings = await import('../web/src/api/mock/eventSettings.ts');
const events = await import('../web/src/api/mock/events.ts');

const now = Date.now();
const lastHour = { from: now - 3_600_000, to: now };
const DEMO_HOST = /^i-0a1b2c3d4e5f6000\d$/;

// ---------------------------------------------------------------- Check Summary

test('check names read as titles, like the server', () => {
  assert.equal(checks.checkDisplayName('datadog.agent.up'), 'Datadog Agent Up');
  assert.equal(checks.checkDisplayName('ntp.in_sync'), 'NTP In Sync');
  assert.equal(checks.checkDisplayName('http.can_connect'), 'HTTP Can Connect');
  assert.equal(checks.statusLabel(2), 'CRITICAL');
  assert.equal(checks.statusClass(null), 'none');
});

test('checks filter on either column and sort both ways', () => {
  const list = [
    { check: 'ntp.in_sync', name: 'NTP In Sync' },
    { check: 'datadog.agent.up', name: 'Datadog Agent Up' },
  ];
  assert.deepEqual(
    checks.filterChecks([...list], '', 'check', 'asc').map((c) => c.check),
    ['datadog.agent.up', 'ntp.in_sync'],
  );
  assert.deepEqual(
    checks.filterChecks([...list], '', 'name', 'desc').map((c) => c.check),
    ['ntp.in_sync', 'datadog.agent.up'],
  );
  assert.deepEqual(
    checks.filterChecks([...list], 'agent', 'check', 'asc').map((c) => c.check),
    ['datadog.agent.up'],
  );
});

test('create monitor prefills a metric monitor on the check', () => {
  const url = new URL(checks.checkMonitorLink('http.can_connect'), 'http://x');
  assert.equal(url.pathname, '/monitors/create/metric');
  assert.equal(url.searchParams.get('query'), 'max:service_check.status{check:http.can_connect} by {host}');
  assert.equal(url.searchParams.get('critical'), '2');
  assert.equal(url.searchParams.get('warning'), '1');
});

test('the mock checks use demo hosts and count groups by their latest status', async () => {
  const list = await checksMock.listServiceChecks();
  assert.ok(list.length >= 5);
  assert.deepEqual(
    list.map((c) => c.check),
    [...list.map((c) => c.check)].sort(),
  );
  const d = await checksMock.serviceCheckDetail('http.can_connect', { from: now - 86_400_000, to: now });
  assert.equal(d.counts.ok + d.counts.warning + d.counts.critical + d.counts.unknown, d.groups.length);
  assert.equal(d.counts.critical, 1);
  assert.equal(d.groups[0].status, 2, 'the worst first');
  assert.ok(d.groups.every((g) => DEMO_HOST.test(g.host) && g.strip.length === d.buckets.length));
  assert.ok(d.latestMessage?.message);
  await assert.rejects(checksMock.serviceCheckDetail('nope', lastHour));
});

// ---------------------------------------------------------------- Derived Metrics

test('a derived name resolves to its formula with the query filters and group-by, like the server', () => {
  const formulas = { 'checkout.error_ratio': '(sum:checkout.errors{status:5xx} by {host}) / (sum:checkout.requests{*} by {host}) * 100' };
  assert.equal(
    derived.expandDerived('avg:checkout.error_ratio{env:prod} by {service}', formulas),
    '((sum:checkout.errors{status:5xx,env:prod} by {service}) / (sum:checkout.requests{env:prod} by {service}) * 100)',
  );
  assert.equal(derived.expandDerived('avg:system.load.1{*}', formulas), 'avg:system.load.1{*}');
  assert.throws(() => derived.expandDerived('avg:b.ratio{*}', { 'a.ratio': 'sum:x{*}', 'b.ratio': 'sum:a.ratio{*} * 2' }), /nested/);
  assert.deepEqual(derived.queryRow('sum:trace.errors{service:billing,!env:qa} by {env}'), { agg: 'sum', metric: 'trace.errors', from: 'service:billing, !env:qa', by: 'env' });
});

test('mock derived metrics validate like the server and graph through the mock engine', async () => {
  const list = await derivedMock.listDerivedMetrics();
  assert.ok(list.some((m) => m.name === 'checkout.error_ratio'));
  await assert.rejects(derivedMock.createDerivedMetric({ name: 'trace.x', queries: ['sum:trace.requests{*}'], expression: 'a' }), /can't start/);
  await assert.rejects(derivedMock.createDerivedMetric({ name: 'x.ratio', queries: ['sum:checkout.error_ratio{*}'], expression: 'a' }), /nested/);
  await assert.rejects(derivedMock.createDerivedMetric({ name: 'x.ratio', queries: ['sum:trace.requests{*}'], expression: 'a / c' }), /formula/i);
  const created = await derivedMock.createDerivedMetric({ name: 'storefront.requests.doubled', queries: ['sum:trace.requests{service:storefront-api}'], expression: 'a * 2' });
  assert.equal(created.formula, '(sum:trace.requests{service:storefront-api}) * 2');
  const [plain] = await engine.queryTimeseries({ queries: ['sum:trace.requests{service:storefront-api}'], range: lastHour, points: 20 });
  const [doubled] = await engine.queryTimeseries({ queries: ['sum:storefront.requests.doubled{*}'], range: lastHour, points: 20 });
  assert.equal(doubled.points.length, plain.points.length);
  doubled.points.forEach(([, v], i) => assert.ok(Math.abs(v - plain.points[i][1] * 2) < 0.05));
  assert.ok(engine.metricCatalog().some((m) => m.name === 'storefront.requests.doubled' && m.kind === 'derived'));
  await assert.rejects(derivedMock.updateDerivedMetric(created.id, { name: 'other.name', queries: created.queries, expression: 'a' }), /renamed/);
  await derivedMock.deleteDerivedMetric(created.id);
  assert.ok(!(await derivedMock.listDerivedMetrics()).some((m) => m.id === created.id));
});

// ---------------------------------------------------------------- Metrics rules

test('rule patterns glob metric names, ignoring case', () => {
  assert.ok(rules.globMatch('kubernetes.*', 'kubernetes.cpu.usage.total'));
  assert.ok(rules.globMatch('*.usage.*', 'Kubernetes.CPU.usage.total'));
  assert.ok(!rules.globMatch('kubernetes.*', 'system.load.1'));
  assert.ok(rules.globMatch('system.load.1', 'system.load.1'));
  assert.ok(!rules.globMatch('system.load.1', 'system.load.15'), 'a dot is literal');
  assert.equal(rules.ruleSummary({ action: 'strip_tags', tags: ['pod_name'] }), 'Strip pod_name');
  assert.deepEqual(rules.parseTags(' env, ,service , env'), ['env', 'service']);
});

test('mock rules validate like the server and drive the overview', async () => {
  const before = await rulesMock.listMetricRules();
  assert.ok(before.some((r) => r.kind === 'indexing' && r.metricsImpacted > 0));
  await assert.rejects(rulesMock.createMetricRule({ name: 'x', kind: 'indexing', metricPattern: 'a.*', action: 'drop', tags: [] }), /indexing controls allow tags/);
  await assert.rejects(rulesMock.createMetricRule({ name: 'x', kind: 'ingestion', metricPattern: 'a.*', action: 'strip_tags', tags: [] }), /at least one tag/);
  await assert.rejects(rulesMock.createMetricRule({ name: 'x', kind: 'ingestion', metricPattern: "a'; --", action: 'drop', tags: [] }), /pattern/);
  const drop = await rulesMock.createMetricRule({ name: 'Drop restarts', kind: 'ingestion', metricPattern: 'kubernetes.containers.restart*', action: 'drop', tags: ['ignored'] });
  assert.deepEqual(drop.tags, []);
  assert.equal(drop.metricsImpacted, 1);
  const o = await rulesMock.metricRulesOverview();
  const k8s = o.filtered.find((m) => m.name === 'kubernetes.cpu.usage.total');
  assert.ok(k8s && k8s.indexed <= k8s.ingested && k8s.rules.includes('Kubernetes workload tags'));
  assert.ok(o.filtered.some((m) => m.rules.includes('Drop restarts')));
  assert.ok(o.top.length > 0 && o.top.every((m, i) => i === 0 || o.top[i - 1].ingested >= m.ingested));
  await rulesMock.deleteMetricRule(drop.id);
});

// ---------------------------------------------------------------- Event Management settings

const ev = (timestamp, title, tags, extra = {}) => ({
  timestamp,
  title,
  text: 'Back-off restarting failed container',
  alertType: 'warning',
  priority: 'normal',
  source: 'kubernetes',
  host: 'i-0a1b2c3d4e5f60001',
  aggregationKey: '',
  tags,
  ...extra,
});

test('the event matcher follows the search syntax, like the server', () => {
  const e = ev(0, 'Pod checkout-7d9 restarted', ['kube_namespace:shop', 'env:prod']);
  assert.ok(correlation.eventMatches('source:kubernetes kube_namespace:shop', e));
  assert.ok(correlation.eventMatches('-env:staging CHECKOUT-7d9', e));
  assert.ok(!correlation.eventMatches('-kube_namespace:shop', e));
  assert.ok(!correlation.eventMatches('source:Kubernetes', e));
  assert.ok(!correlation.eventMatches('-restarted', e));
});

test('correlators group by their keys within their window; maintenance windows suppress', () => {
  const MIN = 60_000;
  const c = { id: 'c1', name: 'Shop pods', query: 'source:kubernetes', groupBy: ['kube_namespace'], windowMinutes: 10 };
  const out = correlation.correlateEvents(
    [
      ev(0, 'Pod a', ['kube_namespace:shop']),
      ev(5 * MIN, 'Pod b', ['kube_namespace:shop']),
      ev(6 * MIN, 'Pod c', ['kube_namespace:billing']),
      ev(30 * MIN, 'Pod a', ['kube_namespace:shop']),
      ev(31 * MIN, 'Disk full', [], { source: 'nagios' }),
    ],
    [c],
    [{ name: 'Billing upgrade', query: 'kube_namespace:billing', startsAt: 0, endsAt: 10 * MIN }],
  );
  assert.deepEqual(
    out.map((g) => [correlation.groupTitle(g), g.events.length, g.maintenance]),
    [
      ['Shop pods: shop', 2, null],
      ['Shop pods: billing', 1, 'Billing upgrade'],
      ['Shop pods: shop', 1, null],
      ['Disk full', 1, null],
    ],
  );
  assert.notEqual(out[0].key, out[2].key);
  assert.ok(correlation.inView({ state: 'open', maintenance: 'x' }, 'suppressed'));
  assert.ok(!correlation.inView({ state: 'open', maintenance: 'x' }, 'open'));
  assert.ok(correlation.inView({ state: 'closed', maintenance: null }, 'closed'));
});

test('the mock inbox applies correlators, maintenance windows and attribute values', async () => {
  const range = { from: now - 4 * 3_600_000, to: now };
  const before = await events.workItems({ range });
  assert.ok(before.suppressed > 0, 'the seeded maintenance window covers the billing alerts');
  assert.equal(before.open + before.closed + before.suppressed, before.items.length);
  const correlator = await eventSettings.createCorrelator({ name: 'Kubernetes by namespace', query: 'source:Kubernetes', groupBy: ['kube_namespace'], windowMinutes: 60 });
  const after = await events.workItems({ range });
  const grouped = after.items.filter((i) => i.correlator === 'Kubernetes by namespace');
  assert.ok(grouped.length >= 1 && grouped[0].title.startsWith('Kubernetes by namespace: shop'));
  const [severity] = await eventSettings.listAttributes();
  await eventSettings.setValue(grouped[0].id, severity.id, severity.options[0]);
  await assert.rejects(eventSettings.setValue(grouped[0].id, severity.id, 'SEV-9'), /isn't a value/);
  const again = await events.workItems({ range });
  assert.equal(again.items.find((i) => i.id === grouped[0].id)?.attributes[severity.id], severity.options[0]);
  await assert.rejects(eventSettings.createCorrelator({ name: 'kubernetes BY namespace', windowMinutes: 10 }), /already exists/);
  await assert.rejects(eventSettings.createCorrelator({ name: 'x', groupBy: ['bad key'], windowMinutes: 10 }), /group by/i);
  await assert.rejects(eventSettings.createWindow({ name: 'x', startsAt: 2, endsAt: 1 }), /ends after/);
  await eventSettings.deleteCorrelator(correlator.id);
});
