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
const wd = await import('../web/src/lib/watchdog.ts');
const mock = await import('../web/src/api/mock/watchdog.ts');

const MIN = 60_000;
const now = Date.UTC(2026, 9, 28, 12, 0);

/** An alert shaped like the server's, with defaults. */
function alert(over) {
  const base = {
    id: 'error_rate:service:order-service:1',
    category: 'apm',
    alertType: 'error_rate',
    status: 'ongoing',
    title: 'Error rate increase on service:order-service',
    scope: ['service:order-service'],
    service: 'order-service',
    host: null,
    team: 'checkout',
    unit: 'percent',
    start: now - 30 * MIN,
    end: now - 5 * MIN,
    baseline: 0.4,
    peak: 18.5,
    points: [],
    monitor: { monitorType: 'apm', query: 'avg:trace.error_rate{service:order-service}', name: 'Error rate is high on order-service', critical: 5.4 },
  };
  return { ...base, ...over };
}

const alerts = [
  alert({}),
  alert({
    id: 'cpu:host:i-0a1b2c3d4e5f60003:2',
    category: 'infrastructure',
    alertType: 'cpu',
    status: 'resolved',
    title: 'CPU usage increase on host:i-0a1b2c3d4e5f60003',
    scope: ['host:i-0a1b2c3d4e5f60003'],
    service: null,
    host: 'i-0a1b2c3d4e5f60003',
    team: null,
  }),
  alert({
    id: 'error_logs:service:billing:3',
    category: 'logs',
    alertType: 'error_logs',
    status: 'expired',
    title: 'Error logs increase on service:billing',
    scope: ['service:billing'],
    service: 'billing',
    team: 'payments',
    unit: 'logs',
    monitor: { monitorType: 'log', query: 'logs("service:billing status:error") by {service}', name: 'Error log volume is high on billing', critical: 23 },
  }),
  alert({ id: 'latency:service:order-service:4', alertType: 'latency', status: 'resolved', title: 'Latency increase on service:order-service', unit: 'ms' }),
];

test('free text, facet and scope terms filter alerts', () => {
  const ids = (q) => wd.filterAlerts(alerts, q).map((a) => a.id.split(':')[0]);
  assert.deepEqual(ids(''), ['error_rate', 'cpu', 'error_logs', 'latency']);
  assert.deepEqual(ids('category:apm'), ['error_rate', 'latency']);
  assert.deepEqual(ids('category:(apm OR logs) status:resolved'), ['latency']);
  assert.deepEqual(ids('-category:apm'), ['cpu', 'error_logs']);
  assert.deepEqual(ids('type:cpu'), ['cpu']);
  assert.deepEqual(ids('service:order-service'), ['error_rate', 'latency']);
  assert.deepEqual(ids('host:i-0a1b2c3d4e5f6*'), ['cpu']);
  assert.deepEqual(ids('team:payments'), ['error_logs']);
  assert.deepEqual(ids('latency'), ['latency'], 'words match the title');
  assert.deepEqual(ids('BILLING'), ['error_logs'], 'case-insensitive');
});

test('facets list every known value with counts that ignore their own selection', () => {
  const facets = wd.alertFacets(alerts, 'category:apm');
  assert.deepEqual(
    facets.map((f) => [f.key, f.group]),
    [
      ['category', 'ALL ALERTS'],
      ['type', 'ALL ALERTS'],
      ['status', 'ALL ALERTS'],
    ],
  );
  const category = Object.fromEntries(facets[0].values.map((v) => [v.value, v.count]));
  assert.deepEqual(category, { apm: 2, infrastructure: 1, logs: 1, third_party: 0 }, 'the category facet counts as if no category were picked');
  const status = Object.fromEntries(facets[2].values.map((v) => [v.value, v.count]));
  assert.deepEqual(status, { ongoing: 1, expired: 0, resolved: 1 }, 'other facets count within the selection');
  assert.equal(wd.valueLabel('category', 'third_party'), 'Third Party');
  assert.equal(wd.valueLabel('category', 'apm'), 'APM');
  assert.equal(wd.valueLabel('type', 'error_logs'), 'Error logs');
  assert.equal(wd.valueLabel('status', 'ongoing'), 'Ongoing');
});

test('teams are counted from the alerts', () => {
  assert.deepEqual(wd.teamCounts(alerts, ''), [
    { value: 'checkout', count: 2 },
    { value: 'payments', count: 1 },
  ]);
});

test('create-monitor links carry the query, name and threshold', () => {
  const link = new URL(wd.monitorLink(alerts[0]), 'http://x');
  assert.equal(link.pathname, '/monitors/create/apm');
  assert.equal(link.searchParams.get('query'), 'avg:trace.error_rate{service:order-service}');
  assert.equal(link.searchParams.get('name'), 'Error rate is high on order-service');
  assert.equal(link.searchParams.get('critical'), '5.4');
  const logs = new URL(wd.monitorLink(alerts[2]), 'http://x');
  assert.equal(logs.pathname, '/monitors/create/log');
  assert.equal(logs.searchParams.get('aggregation'), 'sum');
  assert.equal(logs.searchParams.get('window'), '5', 'error logs are counted per 5-minute bucket');
});

test('values and durations read naturally', () => {
  assert.equal(wd.formatValue(18.456, 'percent'), '18.5%');
  assert.equal(wd.formatValue(640.2, 'ms'), '640 ms');
  assert.equal(wd.formatValue(1534, 'ms'), '1.53 s');
  assert.equal(wd.formatValue(120, 'logs'), '120 logs');
  assert.equal(wd.formatDuration(25 * MIN), '25 min');
  assert.equal(wd.formatDuration(65 * MIN), '1 h 5 min');
  assert.equal(wd.formatDuration(120 * MIN), '2 h');
});

test('related alerts share a service or host', () => {
  assert.deepEqual(
    wd.relatedAlerts(alerts[0], alerts).map((a) => a.alertType),
    ['latency'],
  );
  assert.deepEqual(wd.relatedAlerts(alerts[1], alerts), []);
});

test('the sparkline scales points into the box and the anomaly window', () => {
  const points = [0, 1, 2, 3].map((i) => ({ t: i * 5 * MIN, value: [1, 2, null, 5][i], lower: null, upper: null }));
  const s = wd.sparkline(points, 5 * MIN, 15 * MIN, 100, 20);
  assert.equal(s.path, 'M0,15.4L33.3,11.8M100,1', 'from 0 to the highest value with 1 px padding; gaps start a new segment');
  assert.deepEqual([s.windowX, s.windowWidth], [33.3, 66.7]);
});

test('the mock answers with every category and status, newest ongoing first', async () => {
  const res = await mock.watchdogAlerts(now);
  assert.ok(res.alerts.length >= 6);
  for (const c of ['apm', 'infrastructure', 'logs'])
    assert.ok(
      res.alerts.some((a) => a.category === c),
      c,
    );
  for (const s of ['ongoing', 'resolved', 'expired'])
    assert.ok(
      res.alerts.some((a) => a.status === s),
      s,
    );
  assert.equal(res.alerts[0].status, 'ongoing');
  for (const a of res.alerts) {
    assert.ok(a.points.length > 10, a.id);
    assert.ok(a.points.every((p) => p.t % (5 * MIN) === 0));
    assert.ok(a.start < a.end && a.end <= now);
    assert.ok(a.peak > a.baseline);
    assert.ok(a.monitor.query.includes(a.service ?? a.host));
  }
  assert.ok(
    res.alerts.every((a) => !/[A-Z]{2,}-\d|acme|prod-db/i.test(a.title)),
    'fictional names only',
  );
});
