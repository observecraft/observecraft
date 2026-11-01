import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
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
  load(url, context, next) {
    if (url.endsWith('.json')) return { format: 'module', source: `export default ${readFileSync(new URL(url), 'utf8')};`, shortCircuit: true };
    return next(url, context);
  },
});
const logic = await import('../web/src/api/onCall/logic.ts');
const mock = await import('../web/src/api/mock/onCall.ts');
const identity = await import('../web/src/api/mock/identity.ts');

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const utc = (s) => Date.parse(`${s}:00Z`);
const at = (tz, s) => logic.wallToUtc(tz, logic.parseLocal(s));
const layer = (members, rotationHours, effectiveFrom, restrictions = []) => ({ id: 'l1', name: 'Layer 1', members, rotationHours, effectiveFrom, effectiveUntil: null, restrictions });

test('local times convert across DST like the server (earlier instant, gaps read with the offset before)', () => {
  assert.equal(at('Europe/Berlin', '2026-10-24T09:00'), utc('2026-10-24T07:00'));
  assert.equal(at('Europe/Berlin', '2026-10-26T09:00'), utc('2026-10-26T08:00'));
  assert.equal(at('America/New_York', '2026-03-08T02:30'), utc('2026-03-08T07:30'));
  assert.equal(at('America/New_York', '2026-11-01T01:30'), utc('2026-11-01T05:30'));
  assert.equal(logic.utcToWall('Europe/Berlin', utc('2026-07-01T10:00')), logic.parseLocal('2026-07-01T12:00'));
  assert.ok(logic.isTimeZone('Asia/Tokyo'));
  assert.ok(!logic.isTimeZone('Mars/Olympus'));
});

test('rotations take turns and keep the local handoff time across DST', () => {
  const l = layer(['ana', 'ben', 'cy'], 24, '2026-10-01T09:00');
  assert.deepEqual(
    logic.layerShifts(l, 'UTC', utc('2026-10-01T09:00'), utc('2026-10-05T09:00')).map((s) => s.userId),
    ['ana', 'ben', 'cy', 'ana'],
  );
  const dst = logic.layerShifts(layer(['ana', 'ben'], 24, '2026-10-24T09:00'), 'Europe/Berlin', utc('2026-10-24T07:00'), utc('2026-10-26T08:00'));
  assert.deepEqual(
    dst.map((s) => [s.userId, s.start, s.end]),
    [
      ['ana', utc('2026-10-24T07:00'), utc('2026-10-25T08:00')],
      ['ben', utc('2026-10-25T08:00'), utc('2026-10-26T08:00')],
    ],
  );
  // Handoffs are half-open.
  const weekly = [layer(['ana', 'ben'], 168, '2026-10-05T09:00')];
  assert.deepEqual(logic.onCallAt(weekly, [], null, 'UTC', utc('2026-10-12T09:00') - 1), ['ana']);
  assert.deepEqual(logic.onCallAt(weekly, [], null, 'UTC', utc('2026-10-12T09:00')), ['ben']);
});

test('restrictions, layer precedence, max concurrent and overrides', () => {
  const tz = 'Europe/Berlin';
  const business = layer(['ana', 'ben'], 168, '2026-10-05T00:00', [{ startDay: 'mon', startTime: '09:00', endDay: 'fri', endTime: '17:00' }]);
  assert.deepEqual(logic.onCallAt([business], [], null, tz, at(tz, '2026-10-05T08:00')), []);
  assert.deepEqual(logic.onCallAt([business], [], null, tz, at(tz, '2026-10-07T03:00')), ['ana']);
  assert.deepEqual(logic.onCallAt([business], [], null, tz, at(tz, '2026-10-10T12:00')), []);
  const base = layer(['ana'], 168, '2026-10-05T00:00');
  const weekend = { ...layer(['cy'], 168, '2026-10-05T00:00', [{ startDay: 'sat', startTime: '00:00', endDay: 'mon', endTime: '00:00' }]), id: 'l2' };
  assert.deepEqual(logic.onCallAt([base, weekend], [], null, 'UTC', utc('2026-10-10T12:00')), ['cy', 'ana']);
  assert.deepEqual(logic.onCallAt([base, weekend], [], 1, 'UTC', utc('2026-10-10T12:00')), ['cy']);
  const fin = logic.finalSchedule([base], [{ userId: 'dee', start: utc('2026-10-06T10:00'), end: utc('2026-10-06T14:00') }], null, 'UTC', utc('2026-10-06T00:00'), utc('2026-10-07T00:00'));
  assert.deepEqual(
    fin.map((s) => [s.userIds.join(), s.isOverride, s.start, s.end]),
    [
      ['ana', false, utc('2026-10-06T00:00'), utc('2026-10-06T10:00')],
      ['dee', true, utc('2026-10-06T10:00'), utc('2026-10-06T14:00')],
      ['ana', false, utc('2026-10-06T14:00'), utc('2026-10-07T00:00')],
    ],
  );
  assert.deepEqual(logic.shiftsFor('ana', fin), [
    [utc('2026-10-06T00:00'), utc('2026-10-06T10:00')],
    [utc('2026-10-06T14:00'), utc('2026-10-07T00:00')],
  ]);
});

test('layer validation messages', () => {
  assert.equal(logic.layerProblem(layer(['a'], 24, '2026-10-01T09:00')), null);
  assert.match(logic.layerProblem(layer(['a'], 0, '2026-10-01T09:00')), /1–8760/);
  assert.match(logic.layerProblem(layer(['a'], 24, '')), /start date/);
  assert.match(logic.layerProblem({ ...layer(['a'], 24, '2026-10-01T09:00'), effectiveUntil: '2026-09-01T00:00' }), /after the start/);
  assert.equal(logic.rotationLabel(24), 'Daily');
  assert.equal(logic.rotationLabel(168), 'Weekly');
  assert.equal(logic.rotationLabel(12), 'Every 12 hours');
});

test('month calendar runs Sunday to Saturday and covers the month', () => {
  const oct = logic.monthGrid(2026, 9); // October 2026 starts on a Thursday.
  assert.equal(oct.length, 5);
  assert.deepEqual(
    oct[0].map((d) => [d.day, d.inMonth]),
    [
      [27, false],
      [28, false],
      [29, false],
      [30, false],
      [1, true],
      [2, true],
      [3, true],
    ],
  );
  assert.equal(oct[4][6].day, 31);
  assert.equal(logic.monthGrid(2026, 7).length, 6); // August 2026 spans six weeks.
  assert.ok(logic.monthGrid(2026, 1).every((w) => w.length === 7));
});

test('timeline blocks and day columns', () => {
  assert.deepEqual(logic.timelineBlock(0, 50, 0, 100), { left: 0, width: 50 });
  assert.deepEqual(logic.timelineBlock(-10, 30, 0, 100), { left: 0, width: 30 });
  assert.equal(logic.timelineBlock(100, 120, 0, 100), null);
  const cols = logic.dayColumns('Europe/Berlin', utc('2026-10-24T22:00'), utc('2026-10-27T23:00'));
  assert.deepEqual(cols, [utc('2026-10-24T22:00'), utc('2026-10-25T23:00'), utc('2026-10-26T23:00')]);
  const slot = logic.colorSlot('00000000-0000-0000-0000-000000000001');
  assert.ok(slot >= 1 && slot <= 8);
  assert.equal(logic.colorSlot('x'), logic.colorSlot('x'));
});

test('shifts export as an iCalendar file', () => {
  const ics = logic.shiftsToIcs([{ scheduleId: 's1', scheduleName: 'Checkout, primary', start: utc('2026-11-02T08:00'), end: utc('2026-11-09T08:00') }], utc('2026-10-28T12:00'));
  assert.ok(ics.startsWith('BEGIN:VCALENDAR\r\nVERSION:2.0\r\n'));
  assert.ok(ics.includes('DTSTART:20261102T080000Z\r\nDTEND:20261109T080000Z\r\n'));
  assert.ok(ics.includes('SUMMARY:On call: Checkout\\, primary\r\n'));
  assert.ok(ics.endsWith('END:VCALENDAR\r\n'));
  assert.equal(logic.fmtDuration(90), '2m');
  assert.equal(logic.fmtDuration(5400), '1.5h');
  assert.equal(logic.fmtDuration(null), '—');
  assert.match(logic.defaultScheduleName('Alex Morgan', utc('2026-10-28T12:00')), /^Alex's Schedule – Oct 28, 2026$/);
});

test('empty states: onboarded teams, minute choices, policy problems, analytics range', () => {
  const team = (handle, policy, schedules = []) => ({ handle, name: handle, memberCount: 2, policy, schedules, onCallNow: [], openPages: 0, mine: false });
  const teams = [team('billing', null), team('checkout', { id: 'p1', name: 'Checkout' }), team('platform', null, [{ id: 's1', name: 'Platform 24/7' }])];
  assert.deepEqual(
    logic.onboardedTeams(teams).map((t) => t.handle),
    ['checkout', 'platform'],
  );
  assert.deepEqual(logic.onboardedTeams([team('billing', null)]), []);

  const choices = logic.minuteChoices([5, 60, 1440], 5);
  assert.deepEqual(choices, [
    { value: '5', label: '5 minutes' },
    { value: '60', label: '1 hour' },
    { value: '1440', label: '1 day' },
  ]);
  // A stored value outside the list stays selectable, in order.
  assert.deepEqual(
    logic.minuteChoices([5, 60], 90).map((c) => c.label),
    ['5 minutes', '1 hour', '90 minutes'],
  );
  assert.equal(logic.minuteChoices([1], 1)[0].label, '1 minute');
  assert.equal(logic.minuteChoices([120], 120)[0].label, '2 hours');

  const step = (targets, escalateAfterMinutes = 5) => ({ targets, escalateAfterMinutes });
  const user = { kind: 'user', id: 'u1' };
  assert.equal(logic.policyProblem({ name: '', steps: [step([user])], repeat: 0 }), 'Name the escalation policy');
  assert.equal(logic.policyProblem({ name: 'Checkout', steps: [], repeat: 0 }), 'Add at least one step');
  assert.equal(logic.policyProblem({ name: 'Checkout', steps: [step([user]), step([])], repeat: 0 }), 'Add a recipient to step 2');
  assert.equal(logic.policyProblem({ name: 'Checkout', steps: [step([user], 0)], repeat: 0 }), 'Step 1 must escalate after at least 1 minute');
  assert.equal(logic.policyProblem({ name: 'Checkout', steps: [step([user])], repeat: 11 }), 'Repeat at most 10 times');
  assert.equal(logic.policyProblem({ name: ' Checkout ', steps: [step([user])], repeat: 0 }), null);

  // Analytics accept at most 92 days: longer ranges keep their end.
  assert.deepEqual(logic.clampRange({ from: 0, to: 100 * DAY }, 92), { from: 8 * DAY, to: 100 * DAY });
  assert.deepEqual(logic.clampRange({ from: DAY, to: 8 * DAY }, 92), { from: DAY, to: 8 * DAY });
});

test('mock: demo teams have schedules, policies and routing', async () => {
  mock.resetOnCallMock();
  const teams = await mock.listTeams();
  const byHandle = Object.fromEntries(teams.map((t) => [t.handle, t]));
  for (const h of ['checkout', 'payments', 'platform']) {
    assert.ok(byHandle[h].policy, `${h} has a policy`);
    assert.ok(byHandle[h].schedules.length >= 1, `${h} has schedules`);
  }
  assert.ok(byHandle.checkout.onCallNow.length >= 1);
  const schedules = await mock.listSchedules();
  assert.ok(schedules.every((s) => s.members.every((m) => m.email.endsWith('@example.com'))));
  const policies = await mock.listPolicies();
  assert.deepEqual(policies.find((p) => p.name === 'Checkout escalation').usedBy, ['checkout']);
  // The signed-in demo user is on the platform rotation.
  const now = Date.now();
  const mine = await mock.myShifts({ from: now, to: now + 31 * DAY });
  assert.ok(mine.length >= 1 && mine.every((s) => s.scheduleName === 'Platform 24/7'));
});

test('mock: a page notifies the policy, escalates, and acknowledging stops it', async () => {
  mock.resetOnCallMock();
  const page = await mock.createPage({ title: 'Checkout error rate above 5%', team: 'checkout', description: 'order-service 5xx' });
  assert.equal(page.status, 'triggered');
  assert.equal(page.policy.name, 'Checkout escalation');
  assert.equal(page.responders.length, 1);
  const detail = await mock.getPage(page.id);
  assert.deepEqual(
    detail.timeline.map((e) => e.kind),
    ['triggered', 'notified'],
  );
  const counts = (await mock.searchPages({ status: 'active' })).counts;
  assert.ok(counts.triggered >= 1 && counts.active >= counts.triggered);
  const acked = await mock.acknowledgePage(page.id);
  assert.equal(acked.status, 'acknowledged');
  assert.equal(acked.nextEscalationAt, null);
  await assert.rejects(() => mock.acknowledgePage(page.id), /already acknowledged/);
  const resolved = await mock.resolvePage(page.id);
  assert.equal(resolved.status, 'resolved');
  await assert.rejects(() => mock.reassignPage(page.id, { team: 'payments' }), /resolved/);
  // A team without a policy pages every member.
  await mock.setRouting('checkout', null);
  const all = await mock.createPage({ title: 'Everyone', team: 'checkout' });
  const roster = identity.mockTeamRoster().find((t) => t.handle === 'checkout').members;
  assert.equal(all.responders.length, roster.length);
  await assert.rejects(() => mock.createPage({ title: 'x' }), /Page a team/);
});

test('mock: schedules validate, preview and take overrides; analytics add up', async () => {
  mock.resetOnCallMock();
  await assert.rejects(() => mock.createSchedule({ name: 'x', timeZone: 'Mars/Base', layers: [] }), /time zone/);
  const me = identity.ME_ID;
  const s = await mock.createSchedule({ name: "Alex's Schedule", timeZone: 'UTC', layers: [layer([me], 24, '2026-10-01T00:00')] });
  const range = { from: utc('2026-10-10T00:00'), to: utc('2026-10-11T00:00') };
  const o = await mock.createOverride(s.id, { userId: me, start: utc('2026-10-10T06:00'), end: utc('2026-10-10T07:00') });
  const shifts = await mock.scheduleShifts(s.id, range);
  assert.deepEqual(
    shifts.finalShifts.map((f) => f.isOverride),
    [false, true, false],
  );
  assert.equal(shifts.overrides[0].id, o.id);
  const preview = await mock.previewSchedule({ schedule: { name: 'p', timeZone: 'UTC', layers: [layer([me], 12, '2026-10-01T00:00')] }, range });
  assert.equal(preview.layers[0].shifts.length, 2);
  await mock.deleteOverride(s.id, o.id);
  await assert.rejects(() => mock.deleteOverride(s.id, o.id), /not found/);
  const now = Date.now();
  const a = await mock.analytics({ range: { from: now - 30 * DAY, to: now + HOUR }, timeZone: 'Europe/Berlin' });
  assert.equal(
    a.byHour.reduce((x, y) => x + y, 0),
    a.total,
  );
  assert.equal(
    a.perDay.reduce((x, d) => x + d.count, 0),
    a.total,
  );
  assert.equal(
    a.teams.reduce((x, t) => x + t.count, 0),
    a.total,
  );
  assert.ok(a.mttaSeconds > 0);
});

test('mock: profiles choose channels per urgency', async () => {
  mock.resetOnCallMock();
  const p = await mock.getProfile();
  assert.equal(p.configured, false);
  assert.deepEqual(p.highUrgency, ['email']);
  await assert.rejects(() => mock.updateProfile({ highUrgency: ['slack-nope'], lowUrgency: ['email'] }), /Unknown channel/);
  await assert.rejects(() => mock.updateProfile({ highUrgency: [], lowUrgency: ['email'] }), /at least one/);
  const saved = await mock.updateProfile({ highUrgency: ['email', 'slack-platform-oncall'], lowUrgency: ['email'] });
  assert.equal(saved.configured, true);
  assert.deepEqual(
    (await mock.testProfile()).map((r) => r.channel),
    ['email', 'slack-platform-oncall'],
  );
});
