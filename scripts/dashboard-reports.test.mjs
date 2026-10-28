import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
// The dashboards mock imports JSON seeds the way Vite does (no import attribute): add it.
registerHooks({
  resolve(specifier, context, next) {
    if (specifier.endsWith('.json')) return { ...next(specifier, context), importAttributes: { type: 'json' } };
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
});

const mock = await import('../web/src/api/mock/dashboardReports.ts');
const { ME_ID } = await import('../web/src/api/mock/identity.ts');
const dashboards = await import('../web/src/api/mock/dashboards.ts');

const billing = async () =>
  (await dashboards.searchDashboards({ query: 'Billing', preset: 'custom', listId: null, team: null, sort: { field: 'title', dir: 'asc' }, offset: 0, limit: 5 })).dashboards.find(
    (d) => d.title === 'Billing',
  );

const input = (dashboardId, extra = {}) => ({
  name: 'Billing weekly',
  dashboardId,
  templateValues: { env: 'prod' },
  timeFrame: '1w',
  schedule: { frequency: 'weekly', time: '09:00', weekday: 1, monthDay: null, timezone: 'Europe/Paris' },
  recipients: ['Ops@Example.com', '@slack-checkout-alerts'],
  message: 'Weekly numbers for the billing team.',
  ...extra,
});

test('options: email is off in mock mode; channels and time frames are listed', async () => {
  const o = await mock.reportOptions();
  assert.equal(o.emailEnabled, false);
  assert.ok(o.channels.includes('slack-checkout-alerts'));
  assert.deepEqual(
    o.timeFrames.map((t) => t.key),
    ['1h', '4h', '1d', '2d', '1w', '1mo'],
  );
});

test('seeded reports reference demo dashboards', async () => {
  const all = await mock.listReports();
  assert.ok(all.length >= 2);
  assert.ok(all.some((r) => r.dashboardTitle === 'Billing'));
  assert.ok(all.every((r) => r.recipients.every((x) => x.endsWith('@example.com') || /^(slack|webhook)-/.test(x))));
  assert.ok(all.some((r) => r.author?.id === ME_ID));
});

test('create validates and normalizes; update, subscribe and delete', async () => {
  const d = await billing();
  const r = await mock.createReport(input(d.id));
  assert.deepEqual(r.recipients, ['ops@example.com', 'slack-checkout-alerts']);
  assert.equal(r.scheduleLabel, 'Weekly on Monday at 09:00 (Europe/Paris)');
  assert.equal(r.dashboardTitle, 'Billing');
  assert.equal(r.author?.id, ME_ID);
  assert.ok(r.nextRunAt > Date.now());
  await assert.rejects(mock.createReport(input(d.id, { recipients: ['not an address'] })), /isn't an email address or a channel handle/);
  await assert.rejects(mock.createReport(input(d.id, { timeFrame: '3y' })), /Unknown time frame/);
  await assert.rejects(mock.createReport(input(d.id, { schedule: { frequency: 'weekly', time: '09:00', weekday: null, monthDay: null, timezone: 'UTC' } })), /weekday/);
  await assert.rejects(mock.createReport(input('nope')), /Dashboard nope not found/);

  const u = await mock.updateReport(r.id, input(d.id, { name: 'Billing daily', schedule: { frequency: 'daily', time: '07:30', weekday: null, monthDay: null, timezone: 'UTC' } }));
  assert.equal(u.scheduleLabel, 'Daily at 07:30 (UTC)');
  assert.equal((await mock.setReportSubscription(r.id, true)).subscribed, true);
  assert.equal((await mock.setReportSubscription(r.id, true)).subscriberCount, 1);
  assert.equal((await mock.setReportSubscription(r.id, false)).subscribed, false);
  await mock.deleteReport(r.id);
  assert.ok(!(await mock.listReports()).some((x) => x.id === r.id));
});

test('preview summarizes timeseries widgets with current, min and max; send reports email as not sent', async () => {
  const d = await billing();
  const r = await mock.createReport(input(d.id));
  const p = await mock.previewReport(r.id);
  assert.equal(p.subject, '[Report] Billing · Past 1 Week');
  const ts = p.widgets.find((w) => w.title === 'API Gateway count metric');
  assert.ok(ts.series.length > 0);
  for (const s of ts.series) assert.ok(s.min <= s.current && s.current <= s.max, JSON.stringify(s));
  assert.ok(
    p.widgets.some((w) => w.series.length === 0 && !w.error),
    'other widgets are listed',
  );
  assert.match(p.body, /Open the dashboard: .*\/dashboards\/.*time=1w.*tpl_var_env=prod/);
  const sent = await mock.sendReport(r.id);
  assert.deepEqual(
    sent.map((s) => [s.recipient, s.status]),
    [
      ['ops@example.com', 'not_sent'],
      ['slack-checkout-alerts', 'sent'],
    ],
  );
  assert.match(sent[0].error, /Email isn't configured/);
  const after = (await mock.listReports()).find((x) => x.id === r.id);
  assert.equal(after.lastStatus, 'partial');
  assert.ok(after.lastSentAt);
  await mock.deleteReport(r.id);
});
