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
const mon = await import('../web/src/api/mock/monitors.ts');
const ci = await import('../web/src/api/mock/ci.ts');

const MIN = 60_000;
const DAY = 24 * 60 * MIN;
const now = Date.now();
const lastHour = { from: now - 60 * MIN, to: now };
const lastDay = { from: now - DAY, to: now };
const lastWeek = { from: now - 7 * DAY, to: now };

const SEARCH = { query: '', statuses: [], muted: [], priorities: [], types: [], drafts: [], recipients: [], creators: [], teams: [], sort: null, offset: 0, limit: 200 };
const TRIGGERED = { query: '', statuses: [], muted: [], priorities: [], drafts: [], triggeredWithin: null, offset: 0, limit: 200 };
const DOWNTIMES = { query: '', active: [], recurring: [], creators: [], includeCanceled: false, offset: 0, limit: 200 };
const SEVERITY = { alert: 3, warn: 2, nodata: 1, ok: 0 };

const options = (critical, warning = null) => ({
  detection: 'threshold',
  timeAggregation: 'avg',
  windowMinutes: 5,
  comparator: 'above',
  thresholds: { critical, warning, criticalRecovery: null, warningRecovery: null },
  notifyNoData: false,
  noDataMinutes: 10,
  changeMode: null,
  changeMinutes: null,
});

const sum = (facet) => facet.reduce((n, f) => n + f.count, 0);

test('seeded monitors come from the recommended templates and have consistent statuses', () => {
  const page = mon.searchMonitors(SEARCH);
  assert.ok(page.total >= 12, `${page.total} monitors`);
  const names = page.monitors.map((m) => m.name);
  assert.ok(names.includes('CPU usage is high on {{host.name}}'));
  assert.ok(names.includes('[Kubernetes] Monitor Kubernetes Deployments Replica Pods'));
  assert.equal(sum(page.facets.status), page.total);
  for (const s of ['alert', 'ok', 'nodata'])
    assert.ok(
      page.monitors.some((m) => m.status === s),
      `a monitor in ${s}`,
    );
  // Default sort: worst status first.
  const sev = page.monitors.map((m) => SEVERITY[m.status]);
  assert.deepEqual(
    sev,
    [...sev].sort((a, b) => b - a),
  );
  for (const item of page.monitors) {
    const d = mon.getMonitor(item.id);
    assert.equal(d.status, item.status);
    const worst = d.groups.reduce((w, g) => (SEVERITY[g.status] > SEVERITY[w] ? g.status : w), d.groups.length ? 'ok' : 'nodata');
    assert.equal(d.status, worst, `${d.name}: overall status is the worst group's`);
    assert.equal(item.groupCounts.alert, d.groups.filter((g) => g.status === 'alert').length);
  }
  assert.throws(() => mon.getMonitor(999_999), /not found/);
});

test('a created monitor is evaluated and listed with its status', () => {
  const d = mon.createMonitor({
    name: 'Checkout hosts are busy',
    type: 'metric',
    query: 'avg:system.cpu.user{*} by {host}',
    options: options(70, 50),
    message: '{{#is_alert}}@alex.morgan@example.com @slack-checkout-alerts{{/is_alert}}',
    priority: 2,
    tags: ['service:order-service', 'env:prod'],
    teams: ['checkout'],
  });
  assert.ok(d.id > 0);
  assert.ok(d.groups.length > 0, 'one group per host');
  assert.equal(d.query, 'avg(last_5m):avg:system.cpu.user{*} by {host} > 70');
  assert.equal(d.metricQuery, 'avg:system.cpu.user{*} by {host}');
  assert.deepEqual(d.recipients, ['alex.morgan@example.com', 'slack-checkout-alerts']);
  assert.equal(d.creator.name, 'Alex Morgan');
  const listed = mon.searchMonitors({ ...SEARCH, query: 'checkout hosts' });
  assert.equal(listed.total, 1);
  assert.equal(listed.monitors[0].id, d.id);
  assert.equal(listed.monitors[0].status, d.status);

  assert.throws(() => mon.createMonitor({ name: ' ', type: 'metric', query: 'avg:system.cpu.user{*}', options: options(1) }), /name/);
  assert.throws(() => mon.createMonitor({ name: 'x', type: 'metric', query: 'avg:system.cpu.user{*} -', options: options(1) }), /invalid query/);
  assert.throws(() => mon.createMonitor({ name: 'x', type: 'metric', query: 'avg:system.cpu.user{*}', options: options(1, 5) }), /warning threshold/);

  const updated = mon.updateMonitor(d.id, {
    name: 'Checkout hosts are very busy',
    type: 'metric',
    query: 'avg:system.cpu.user{*} by {host}',
    options: options(99.5),
    message: '',
    tags: [],
    teams: [],
  });
  assert.equal(updated.name, 'Checkout hosts are very busy');
  assert.equal(mon.getMonitor(d.id).query, 'avg(last_5m):avg:system.cpu.user{*} by {host} > 99.5');
  const clone = mon.cloneMonitor(d.id);
  assert.equal(clone.name, '[Clone] Checkout hosts are very busy');
  assert.equal(mon.bulkMonitors([clone.id], 'delete').affected, 1);
  assert.throws(() => mon.getMonitor(clone.id), /not found/);
});

test('muting creates a downtime; the monitor and its groups show as muted until unmuted', () => {
  const target = mon.searchMonitors({ ...SEARCH, statuses: ['alert'] }).monitors.find((m) => !m.muted);
  assert.ok(target, 'an unmuted alerting monitor');
  assert.equal(mon.bulkMonitors([target.id], 'mute', 60).affected, 1);
  const muted = mon.searchMonitors({ ...SEARCH, muted: [true] }).monitors.find((m) => m.id === target.id);
  assert.ok(muted?.muted);
  assert.ok(Math.abs(muted.mutedUntil - (Date.now() + 60 * MIN)) < 5_000);
  assert.ok(mon.getMonitor(target.id).groups.every((g) => g.muted));
  const triggered = mon.searchTriggered({ ...TRIGGERED, query: target.name });
  assert.ok(triggered.groups.length > 0 && triggered.groups.every((g) => g.muted));
  const dt = mon.searchDowntimes(DOWNTIMES).downtimes.find((x) => x.monitorId === target.id);
  assert.equal(dt.status, 'active');
  assert.equal(dt.monitorName, target.name);
  assert.deepEqual(dt.scope, ['*']);

  mon.bulkMonitors([target.id], 'unmute');
  assert.equal(
    mon.searchMonitors({ ...SEARCH, muted: [true] }).monitors.find((m) => m.id === target.id),
    undefined,
  );
  assert.equal(
    mon.searchDowntimes(DOWNTIMES).downtimes.find((x) => x.id === dt.id),
    undefined,
  );
  assert.equal(mon.searchDowntimes({ ...DOWNTIMES, includeCanceled: true }).downtimes.find((x) => x.id === dt.id).status, 'canceled');
});

test('a scoped downtime mutes only the groups it covers', () => {
  const d = mon.getMonitor(mon.searchMonitors({ ...SEARCH, query: 'CPU usage is high' }).monitors[0].id);
  const host = d.groups[0].group.split(':')[1];
  const dt = mon.createDowntime({ monitorId: d.id, scope: [`host:${host}`], message: 'Kernel patching' });
  const after = mon.getMonitor(d.id);
  assert.equal(after.muted, false, 'a scoped downtime does not mute the whole monitor');
  assert.deepEqual(
    after.groups.filter((g) => g.muted).map((g) => g.group),
    [`host:${host}`],
  );
  mon.cancelDowntime(dt.id);
  assert.ok(mon.getMonitor(d.id).groups.every((g) => !g.muted));
  assert.throws(() => mon.cancelDowntime(dt.id), /not found/);
  assert.throws(() => mon.createDowntime({ scope: ['web-1'] }), /key:value/);
});

test('search filters by words, tags and facets; facet counts ignore their own filter', () => {
  const all = mon.searchMonitors(SEARCH);
  const byTag = mon.searchMonitors({ ...SEARCH, query: 'service:billing' });
  assert.ok(byTag.total > 0);
  assert.ok(byTag.monitors.every((m) => m.tags.includes('service:billing')));
  const alerts = mon.searchMonitors({ ...SEARCH, statuses: ['alert'] });
  assert.ok(alerts.monitors.every((m) => m.status === 'alert'));
  assert.deepEqual(alerts.facets.status, all.facets.status, 'the status facet ignores the status filter');
  assert.equal(sum(alerts.facets.type), alerts.total);
  const apm = mon.searchMonitors({ ...SEARCH, types: ['apm'] });
  assert.ok(apm.total > 0 && apm.monitors.every((m) => m.type === 'apm'));
  const page = mon.searchMonitors({ ...SEARCH, sort: 'name', limit: 5, offset: 5 });
  assert.equal(page.monitors.length, 5);
  assert.equal(page.total, all.total);
});

test('triggered lists the non-OK groups of every monitor', () => {
  const page = mon.searchTriggered(TRIGGERED);
  assert.ok(page.total > 0);
  for (const g of page.groups) {
    assert.notEqual(g.status, 'ok');
    const d = mon.getMonitor(g.monitorId);
    assert.equal(d.groups.find((x) => x.group === g.group).status, g.status);
  }
  const alerts = mon.searchTriggered({ ...TRIGGERED, statuses: ['alert'] });
  assert.ok(alerts.groups.every((g) => g.status === 'alert'));
  assert.equal(sum(alerts.facets.status), page.total);
});

test('previews backtest a metric query over mock series', () => {
  const p = mon.previewMonitor('metric', 'avg:system.cpu.user{*} by {host}', options(70, 50), lastHour);
  assert.ok(p.raw.length > 0 && p.raw.length <= 20);
  assert.ok(p.groups >= p.raw.length);
  assert.deepEqual(
    p.evaluated.map((s) => s.group),
    p.raw.map((s) => s.group),
  );
  for (const s of p.raw) {
    assert.match(s.group, /^host:/);
    assert.ok(s.points.length > 10);
    assert.ok(s.points.every(([t, v]) => t >= lastHour.from && t <= lastHour.to && typeof v === 'number'));
  }
  assert.ok(p.evaluated[0].points.some(([, v]) => v !== null));
  assert.equal(typeof p.transitions.alert, 'number');
  // An unknown metric has no data rather than failing.
  assert.equal(mon.previewMonitor('metric', 'avg:no.such.metric{*}', options(1), lastHour).groups, 0);
  assert.throws(() => mon.previewMonitor('metric', 'avg:system.cpu.user{*}', options(1), { from: now - 8 * DAY, to: now }), /week/);
});

test('previews evaluate formulas between queries', () => {
  const single = mon.previewMonitor('metric', 'avg:system.cpu.user{*} by {host}', options(1000), lastHour);
  const doubled = mon.previewMonitor('metric', 'avg:system.cpu.user{*} by {host} * 2', options(1000), lastHour);
  assert.equal(doubled.groups, single.groups);
  const a = single.raw[0];
  const b = doubled.raw.find((s) => s.group === a.group);
  assert.deepEqual(
    b.points.map(([t, v]) => [t, Math.round(v * 100)]),
    a.points.map(([t, v]) => [t, Math.round(v * 200)]),
  );
  const k8s = mon.previewMonitor(
    'metric',
    'avg:kubernetes_state.deployment.replicas_desired{*} by {kube_cluster_name,kube_namespace,kube_deployment} - avg:kubernetes_state.deployment.replicas_available{*} by {kube_cluster_name,kube_namespace,kube_deployment}',
    options(2),
    lastHour,
  );
  assert.ok(k8s.groups > 0);
  assert.match(k8s.raw[0].group, /^kube_cluster_name:.*,kube_deployment:.*,kube_namespace:/);
  const mem = mon.previewMonitor('metric', '100 - avg:system.mem.pct_usable{*} by {host} * 100', options(90, 80), lastHour);
  assert.ok(mem.groups > 0);
  assert.ok(mem.raw.every((s) => s.points.every(([, v]) => v >= 0 && v <= 100)));
  const apm = mon.previewMonitor('apm', 'avg:trace.error_rate{env:prod} by {service}', options(5, 2), lastHour);
  assert.ok(apm.raw.some((s) => s.group === 'service:order-service'));
  const logs = mon.previewMonitor('log', 'logs("status:error") by {service}', options(100), lastHour);
  assert.ok(logs.raw.some((s) => s.group === 'service:billing'));
});

test('history replays transitions into status segments', () => {
  const item = mon.searchMonitors({ ...SEARCH, statuses: ['alert'] }).monitors[0];
  const h = mon.monitorHistory(item.id, lastWeek);
  const times = h.events.map((e) => e.createdAt);
  assert.deepEqual(
    times,
    [...times].sort((a, b) => b - a),
    'newest first',
  );
  for (const s of h.segments) assert.ok(s.from < s.to && s.from >= lastWeek.from && s.to <= Date.now());
  const d = mon.getMonitor(item.id);
  for (const g of d.groups) {
    const last = h.segments.filter((s) => s.group === g.group).at(-1);
    assert.equal(last.status, g.status, `${g.group} ends in its current status`);
  }
});

test('quality flags monitors without recipients and stuck ones', () => {
  const q = mon.monitorQuality({ creators: [], teams: [], services: [], envs: [] });
  const kinds = q.issues.map((i) => i.kind);
  assert.ok(kinds.includes('missing_recipients'));
  assert.ok(kinds.includes('stuck_no_data'));
  assert.equal(q.healthy + q.toImprove, mon.searchMonitors(SEARCH).total);
  assert.ok(q.services.includes('billing'));
});

test('test notifications report each recipient', () => {
  const out = mon.sendTestNotification({ monitorName: 'x', message: 'Hi @alex.morgan@example.com @slack-checkout-alerts @nobody@elsewhere.org', status: 'alert' });
  assert.deepEqual(
    out.map((d) => [d.recipient, d.status]),
    [
      ['alex.morgan@example.com', 'sent'],
      ['slack-checkout-alerts', 'sent'],
      ['nobody@elsewhere.org', 'unknown_recipient'],
    ],
  );
  assert.throws(() => mon.sendTestNotification({ monitorName: 'x', message: 'no one', status: 'alert' }), /no @recipients/);
});

// ---------------------------------------------------------------- CI Visibility

const ciq = (over = {}) => ({ range: lastDay, level: 'pipeline', query: '', filters: { statuses: [], pipelines: [], repositories: [], branches: [], providers: [], teams: [] }, ...over });

test('pipeline executions list newest first and filter by facets and query terms', () => {
  const page = ci.searchExecutions(ciq(), 'date', 0, 100);
  assert.ok(page.total > 20, `${page.total} pipeline runs in a day`);
  const starts = page.executions.map((e) => e.start);
  assert.deepEqual(
    starts,
    [...starts].sort((a, b) => b - a),
  );
  for (const e of page.executions) {
    assert.match(e.repository, /^example-org\//);
    assert.match(e.traceId, /^[0-9a-f]{32}$/);
    assert.ok(['success', 'error', 'canceled'].includes(e.status));
    assert.ok(['ci', 'deploy', 'terraform plan', 'terraform apply'].includes(e.name));
    assert.ok(e.start >= lastDay.from && e.start <= lastDay.to);
  }
  const failed = ci.searchExecutions(ciq({ filters: { ...ciq().filters, statuses: ['error'] } }), 'date', 0, 500);
  assert.ok(failed.total > 0 && failed.executions.every((e) => e.status === 'error'));
  const billing = ci.searchExecutions(ciq({ query: 'repo:example-org/billing' }), 'date', 0, 500);
  assert.ok(billing.total > 0 && billing.executions.every((e) => e.repository === 'example-org/billing'));
  const notMain = ci.searchExecutions(ciq({ query: '-branch:main' }), 'date', 0, 500);
  assert.ok(notMain.executions.every((e) => e.branch !== 'main'));
  const wild = ci.searchExecutions(ciq({ query: 'branch:feature/*' }), 'date', 0, 500);
  assert.ok(wild.total > 0 && wild.executions.every((e) => e.branch.startsWith('feature/')));
  assert.throws(() => ci.searchExecutions(ciq({ query: 'color:red' }), 'date', 0, 10), /unknown filter/);
  const facets = ci.executionFacets(ciq());
  assert.equal(sum(facets.repository), page.total);
  assert.equal(sum(failed.total ? ci.executionFacets(ciq({ filters: { ...ciq().filters, statuses: ['error'] } })).status : []), page.total, 'the status facet ignores the status filter');
  assert.deepEqual(ci.searchExecutions(ciq(), 'date', 0, 100), page, 'deterministic');
});

test('a pipeline detail is the same run as its execution row, as a span tree', () => {
  const e = ci.searchExecutions(ciq({ query: 'pipeline:ci' }), 'date', 0, 1).executions[0];
  const d = ci.pipelineDetail(e.traceId);
  assert.deepEqual(d.pipeline, e);
  const root = d.spans.filter((s) => s.kind === 'pipeline');
  assert.equal(root.length, 1);
  assert.equal(root[0].spanId, e.spanId);
  assert.equal(root[0].status, e.status);
  const ids = new Map(d.spans.map((s) => [s.spanId, s]));
  const jobs = d.spans.filter((s) => s.kind === 'job');
  assert.ok(jobs.length >= 1);
  for (const j of jobs) assert.equal(j.parentSpanId, e.spanId);
  for (const s of d.spans.filter((x) => x.kind === 'step')) assert.equal(ids.get(s.parentSpanId).kind, 'job');
  for (const t of d.spans.filter((x) => x.kind === 'test')) {
    assert.equal(ids.get(t.parentSpanId).name, 'Run');
    assert.ok(t.testSuite);
  }
  // Job-level rows of the same run.
  const jobRows = ci.searchExecutions(ciq({ level: 'job', query: `sha:${e.sha} pipeline:ci` }), 'date', 0, 100).executions.filter((x) => x.traceId === e.traceId);
  assert.deepEqual(jobRows.map((x) => x.spanId).sort(), jobs.map((j) => j.spanId).sort());
  for (const j of jobRows) assert.equal(j.stage, ids.get(j.spanId).stage);
  if (e.status === 'error') assert.ok(jobs.some((j) => j.status === 'error'));
  assert.throws(() => ci.pipelineDetail('nope'), /invalid trace id/);
});

test('charts, aggregations and overview agree with the executions', () => {
  const total = ci.searchExecutions(ciq(), 'date', 0, 1).total;
  const charts = ci.executionCharts(ciq());
  assert.ok(charts.series.length > 0 && charts.series.length <= 9);
  assert.equal(
    charts.series.reduce((n, s) => n + s.executions.reduce((a, b) => a + b, 0), 0),
    total,
  );
  const aggs = ci.aggregations(ciq());
  assert.equal(
    aggs.reduce((n, a) => n + a.executions, 0),
    total,
  );
  for (const a of aggs) assert.ok(a.p50Ms <= a.p95Ms);
  const o = ci.ciOverview(lastDay);
  assert.equal(o.current.executions, total);
  assert.ok(o.previous.executions > 0);
  assert.ok(o.recentFailures.every((e) => e.status === 'error'));
  assert.equal(o.current.repositories, 8);
  const repos = ci.ciRepositories();
  assert.equal(repos.length, 8);
  assert.ok(repos.find((r) => r.repository === 'example-org/infra-terraform').pipelines.includes('terraform plan'));
});

test('flaky tests passed and failed on the same commit, and keep their state', () => {
  const page = ci.flakyTests(lastWeek, '', []);
  assert.ok(page.tests.length > 0);
  for (const t of page.tests) {
    assert.ok(t.failures > 0 && t.failures < t.runs);
    assert.ok(t.flakyCommits > 0);
    assert.match(t.repository, /^example-org\//);
  }
  const t = page.tests[0];
  ci.setFlakyState(t.repository, t.suite, t.name, 'quarantined');
  const after = ci.flakyTests(lastWeek, '', ['quarantined']);
  assert.deepEqual(
    after.tests.map((x) => x.name),
    [t.name],
  );
  assert.equal(after.tests[0].stateUpdatedBy, 'Alex Morgan');
  assert.equal(after.counts.quarantined, 1);
  assert.equal(after.counts.active, page.tests.length - 1);
  assert.equal(ci.flakyTests(lastWeek, 'no-such-test', []).tests.length, 0);
});

test('DORA metrics come from production deploy jobs', () => {
  const r = ci.doraMetrics(lastWeek, null, []);
  assert.ok(r.summary.deployments > 0);
  assert.equal(
    r.days.reduce((n, d) => n + d.deployments, 0),
    r.summary.deployments,
  );
  assert.deepEqual(r.environments, ['production', 'staging']);
  assert.ok(r.services.some((s) => s.service === 'order-service' && s.repository === 'example-org/order-service'));
  const one = ci.doraMetrics(lastWeek, null, ['billing']);
  assert.ok(one.services.every((s) => s.service === 'billing'));
  assert.ok(r.summary.ratings.deploymentFrequency);
});
