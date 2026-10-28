// In-browser On-Call for mock mode: schedules, escalation policies, team routing, pages with their
// escalation and timeline, profiles and analytics, kept in memory for the session. It follows the
// server's rules (routes/on_call.rs): schedules are computed with the same logic (onCall/logic.ts),
// a page notifies its policy's steps as time passes (escalation runs lazily on read) and teams
// without a policy page every member. The people and teams come from the identity mock.
import { DAY, HOUR, finalSchedule, formatLocal, isTimeZone, layerProblem, layerShifts, onCallAt, shiftsFor, utcToWall, type OverrideSpan } from '../onCall/logic';
import type {
  CreateOnCallOverride,
  CreateOnCallPage,
  OnCallAnalytics,
  OnCallAnalyticsRequest,
  OnCallLayer,
  OnCallMyShift,
  OnCallOverride,
  OnCallPage,
  OnCallPageDetail,
  OnCallPageEvent,
  OnCallPageSearch,
  OnCallPagesResult,
  OnCallPageStatus,
  OnCallPolicy,
  OnCallPolicyInput,
  OnCallPreviewRequest,
  OnCallProfile,
  OnCallProfileInput,
  OnCallRange,
  OnCallSchedule,
  OnCallScheduleInput,
  OnCallScheduleShifts,
  OnCallStep,
  OnCallTeam,
  OnCallTestResult,
  OnCallUser,
  ReassignOnCallPage,
} from '../onCall/types';
import { ME_ID, mockMe, mockPerson, mockTeamRoster } from './identity';

const MIN = 60_000;
const CHANNELS = ['slack-checkout-alerts', 'slack-platform-oncall', 'webhook-oncall'];

interface ScheduleRow {
  id: string;
  name: string;
  timeZone: string;
  team: string | null;
  tags: string[];
  layers: OnCallLayer[];
  maxConcurrent: number | null;
  overrides: { id: string; userId: string; start: number; end: number }[];
  createdAt: number;
  updatedAt: number;
}

interface PolicyRow {
  id: string;
  name: string;
  team: string | null;
  tags: string[];
  steps: OnCallStep[];
  repeat: number;
  resolveAfterMinutes: number | null;
  createdAt: number;
  updatedAt: number;
}

interface PageRow {
  id: number;
  title: string;
  description: string;
  urgency: 'high' | 'low';
  status: OnCallPageStatus;
  team: string | null;
  policyId: string | null;
  source: 'manual' | 'incident' | 'monitor';
  incidentId: number | null;
  monitorId: number | null;
  responders: string[];
  createdBy: string;
  createdAt: number;
  acknowledgedBy: string | null;
  acknowledgedAt: number | null;
  resolvedBy: string | null;
  resolvedAt: number | null;
  step: number;
  round: number;
  nextEscalationAt: number | null;
  resolveAt: number | null;
  timeline: OnCallPageEvent[];
}

interface Db {
  schedules: ScheduleRow[];
  policies: PolicyRow[];
  routing: Record<string, string>;
  pages: PageRow[];
  profiles: Record<string, { highUrgency: string[]; lowUrgency: string[] }>;
  seq: number;
}

let state: Db | null = null;

const fail = (message: string): never => {
  throw new Error(message);
};

const uuid = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `id-${Math.random().toString(36).slice(2)}`);

/** Monday 09:00 (local wall) a number of weeks before now, as "YYYY-MM-DDTHH:MM". */
function mondayWeeksAgo(tz: string, now: number, weeks: number, hour = 9): string {
  const w = utcToWall(tz, now);
  const day = Date.UTC(new Date(w).getUTCFullYear(), new Date(w).getUTCMonth(), new Date(w).getUTCDate());
  const monday = day - ((new Date(day).getUTCDay() + 6) % 7) * DAY;
  return formatLocal(monday - weeks * 7 * DAY + hour * HOUR);
}

function roster(handle: string): string[] {
  return mockTeamRoster().find((t) => t.handle === handle)?.members ?? [];
}

function db(): Db {
  if (state) return state;
  const now = Date.now();
  const checkout = roster('checkout').slice(0, 4);
  const payments = roster('payments').slice(0, 3);
  const platform = [ME_ID, ...roster('platform').filter((id) => id !== ME_ID)].slice(0, 4);
  const layer = (id: string, name: string, members: string[], rotationHours: number, effectiveFrom: string, restrictions: OnCallLayer['restrictions'] = []): OnCallLayer => ({
    id,
    name,
    members,
    rotationHours,
    effectiveFrom,
    effectiveUntil: null,
    restrictions,
  });
  const tzB = 'Europe/Berlin';
  const tzNy = 'America/New_York';
  const sched = (name: string, timeZone: string, team: string, layers: OnCallLayer[], ageDays: number, tags: string[] = []): ScheduleRow => ({
    id: uuid(),
    name,
    timeZone,
    team,
    tags,
    layers,
    maxConcurrent: null,
    overrides: [],
    createdAt: now - ageDays * DAY,
    updatedAt: now - Math.floor(ageDays / 3) * DAY,
  });
  const schedules: ScheduleRow[] = [
    sched('Checkout primary', tzB, 'checkout', [layer('layer-1', 'Weekly rotation', checkout, 168, mondayWeeksAgo(tzB, now, 6))], 62, ['service:order-service']),
    sched('Checkout secondary', tzB, 'checkout', [layer('layer-1', 'Weekly rotation', [...checkout.slice(2), ...checkout.slice(0, 2)], 168, mondayWeeksAgo(tzB, now, 6))], 62),
    sched(
      'Payments business hours',
      tzNy,
      'payments',
      [
        layer('layer-1', 'Business hours', payments, 24, mondayWeeksAgo(tzNy, now, 4), [{ startDay: 'mon', startTime: '09:00', endDay: 'fri', endTime: '18:00' }]),
        layer('layer-2', 'After hours', [...payments].reverse(), 168, mondayWeeksAgo(tzNy, now, 4, 18), [{ startDay: 'fri', startTime: '18:00', endDay: 'mon', endTime: '09:00' }]),
      ],
      40,
      ['service:billing'],
    ),
    sched(
      'Platform 24/7',
      tzB,
      'platform',
      [
        layer('layer-1', 'Primary', platform, 168, mondayWeeksAgo(tzB, now, 3)),
        layer('layer-2', 'Weekend cover', platform.slice(1, 3), 168, mondayWeeksAgo(tzB, now, 3, 0), [{ startDay: 'sat', startTime: '00:00', endDay: 'mon', endTime: '00:00' }]),
      ],
      120,
      ['service:api-gateway'],
    ),
  ];
  const [checkoutPrimary, checkoutSecondary, paymentsHours, platformAll] = schedules;
  if (platform[1]) platformAll.overrides.push({ id: uuid(), userId: platform[1], start: now + 2 * DAY, end: now + 2 * DAY + 8 * HOUR });
  const policy = (name: string, team: string, steps: OnCallStep[], repeat: number, resolveAfterMinutes: number | null, ageDays: number): PolicyRow => ({
    id: uuid(),
    name,
    team,
    tags: [`team:${team}`],
    steps,
    repeat,
    resolveAfterMinutes,
    createdAt: now - ageDays * DAY,
    updatedAt: now - Math.floor(ageDays / 2) * DAY,
  });
  const policies: PolicyRow[] = [
    policy(
      'Checkout escalation',
      'checkout',
      [
        { targets: [{ kind: 'schedule', id: checkoutPrimary.id }], escalateAfterMinutes: 5 },
        { targets: [{ kind: 'schedule', id: checkoutSecondary.id }], escalateAfterMinutes: 15 },
        { targets: [{ kind: 'team', id: 'checkout' }], escalateAfterMinutes: 30 },
      ],
      1,
      null,
      60,
    ),
    policy(
      'Payments escalation',
      'payments',
      [
        { targets: [{ kind: 'schedule', id: paymentsHours.id }], escalateAfterMinutes: 10 },
        ...(payments[0] ? [{ targets: [{ kind: 'user' as const, id: payments[0] }], escalateAfterMinutes: 20 }] : []),
      ],
      2,
      720,
      38,
    ),
    policy(
      'Platform escalation',
      'platform',
      [
        { targets: [{ kind: 'schedule', id: platformAll.id }], escalateAfterMinutes: 5 },
        { targets: [{ kind: 'team', id: 'platform' }], escalateAfterMinutes: 10 },
      ],
      0,
      240,
      110,
    ),
  ];
  state = {
    schedules,
    policies,
    routing: { checkout: policies[0].id, payments: policies[1].id, platform: policies[2].id },
    pages: [],
    profiles: {},
    seq: 1,
  };
  seedPages(state, now);
  return state;
}

/** A month of fictional pages, mostly resolved, a few still open. */
function seedPages(s: Db, now: number): void {
  const titles: [string, string, number | null, 'checkout' | 'payments' | 'platform'][] = [
    ['[Triggered] High error rate on service:order-service', 'monitor', 1, 'checkout'],
    ['[Triggered] Checkout latency p95 above 2s', 'monitor', 2, 'checkout'],
    ['[Triggered] Payment provider timeouts on service:billing', 'monitor', 3, 'payments'],
    ['[Triggered] Disk space low on i-0a1b2c3d4e5f60003', 'monitor', 4, 'platform'],
    ['[Triggered] api-gateway 5xx above 1%', 'monitor', 5, 'platform'],
    ['IR-97: Orders stuck in pending', 'incident', null, 'checkout'],
    ['Card authorizations failing for one region', 'manual', null, 'payments'],
    ['[Triggered] Kubernetes nodes are unavailable', 'monitor', 6, 'platform'],
  ];
  let seed = 7;
  const rand = () => {
    seed = (Math.imul(seed, 1_103_515_245) + 12_345) >>> 0;
    return seed / 4_294_967_296;
  };
  for (let i = 0; i < 34; i++) {
    const [title, source, monitorId, team] = titles[Math.floor(rand() * titles.length)];
    const createdAt = now - Math.floor(rand() * 29 * DAY) - 2 * HOUR;
    const page = newPage(
      s,
      { title, description: source === 'monitor' ? `${title.replace('[Triggered] ', '')}. @oncall-${team}` : '', urgency: rand() < 0.75 ? 'high' : 'low', team, policyId: null },
      source as PageRow['source'],
      'ObserveCraft',
      createdAt,
    );
    page.monitorId = monitorId;
    if (source === 'monitor') page.createdBy = `Monitor: ${title.replace('[Triggered] ', '')}`;
    escalate(s, page, createdAt + MIN);
    const ackAfter = Math.round((1 + rand() * 25) * MIN);
    const resolveAfter = ackAfter + Math.round((10 + rand() * 180) * MIN);
    const who = page.responders[0] ? (mockPerson(page.responders[0])?.name ?? 'Someone') : 'Someone';
    setStatus(page, 'acknowledged', who, createdAt + ackAfter);
    setStatus(page, 'resolved', who, createdAt + resolveAfter);
  }
  const open = (title: string, team: string, agoMin: number, ack: boolean) => {
    const page = newPage(
      s,
      { title, description: `${title.replace('[Triggered] ', '')}. @oncall-${team}`, urgency: 'high', team, policyId: null },
      'monitor',
      `Monitor: ${title.replace('[Triggered] ', '')}`,
      now - agoMin * MIN,
    );
    escalate(s, page, now);
    if (ack && page.responders[0]) setStatus(page, 'acknowledged', mockPerson(page.responders[0])?.name ?? 'Someone', now - (agoMin - 3) * MIN);
  };
  open('[Triggered] High error rate on service:order-service', 'checkout', 9, false);
  open('[Triggered] api-gateway 5xx above 1%', 'platform', 42, true);
}

// ---------------------------------------------------------------- shared

function person(id: string): OnCallUser | null {
  return mockPerson(id) ?? null;
}
const people = (ids: string[]): OnCallUser[] => ids.map(person).filter((p): p is OnCallUser => !!p);

function text(s: string, what: string, max: number): string {
  const t = s.trim();
  if (!t || [...t].length > max) fail(`${what} must be 1–${max} characters`);
  return t;
}

function checkTeam(team: string | null | undefined): string | null {
  const t = (team ?? '').trim();
  if (!t) return null;
  if (!mockTeamRoster().some((x) => x.handle === t)) fail(`Unknown team @${t}`);
  return t;
}

function checkRange(r: OnCallRange, maxDays = 93): void {
  if (r.to <= r.from || r.to - r.from > maxDays * DAY) fail(`The range must be positive and at most ${maxDays} days`);
}

const tags = (list: string[] | undefined): string[] => [...new Set((list ?? []).map((t) => t.trim().toLowerCase()).filter(Boolean))];
const spans = (s: ScheduleRow): OverrideSpan[] => s.overrides.map((o) => ({ userId: o.userId, start: o.start, end: o.end }));

// ---------------------------------------------------------------- schedules

function overrideView(o: ScheduleRow['overrides'][number]): OnCallOverride {
  return { id: o.id, userId: o.userId, userName: person(o.userId)?.name ?? 'Unknown user', start: o.start, end: o.end };
}

function scheduleView(s: ScheduleRow): OnCallSchedule {
  const now = Date.now();
  const members = [...new Set(s.layers.flatMap((l) => l.members))];
  return {
    id: s.id,
    name: s.name,
    timeZone: s.timeZone,
    team: s.team,
    tags: [...s.tags],
    layers: structuredClone(s.layers),
    maxConcurrent: s.maxConcurrent,
    overrides: s.overrides.filter((o) => o.end > now - 31 * DAY).map(overrideView),
    onCallNow: people(onCallAt(s.layers, spans(s), s.maxConcurrent, s.timeZone, now)),
    members: people(members).sort((a, b) => a.name.localeCompare(b.name)),
    createdAt: s.createdAt,
    updatedAt: s.updatedAt,
  };
}

function findSchedule(id: string): ScheduleRow {
  return db().schedules.find((s) => s.id === id) ?? fail('Schedule not found');
}

function checkSchedule(input: OnCallScheduleInput): OnCallScheduleInput {
  const name = text(input.name, 'The name', 200);
  if (!isTimeZone(input.timeZone)) fail(`Unknown time zone ${input.timeZone}`);
  if (input.layers.length > 10) fail('At most 10 layers');
  if (input.maxConcurrent != null && (input.maxConcurrent < 1 || input.maxConcurrent > 10)) fail('Max concurrent responders is 1–10');
  const ids = new Set<string>();
  const layers = input.layers.map((l, i) => {
    const id = l.id && !ids.has(l.id) ? l.id : `layer-${i + 1}`;
    ids.add(id);
    const out = { ...l, id, name: text(l.name, 'A layer name', 200), restrictions: l.restrictions ?? [], effectiveUntil: l.effectiveUntil ?? null };
    const problem = layerProblem(out);
    if (problem) fail(problem);
    for (const m of out.members) if (!person(m)) fail(`Unknown or inactive user ${m}`);
    return out;
  });
  return { ...input, name, team: checkTeam(input.team), tags: tags(input.tags), layers, maxConcurrent: input.maxConcurrent ?? null };
}

export async function listSchedules(): Promise<OnCallSchedule[]> {
  return db()
    .schedules.slice()
    .sort((a, b) => a.name.localeCompare(b.name))
    .map(scheduleView);
}

export async function getSchedule(id: string): Promise<OnCallSchedule> {
  return scheduleView(findSchedule(id));
}

export async function createSchedule(input: OnCallScheduleInput): Promise<OnCallSchedule> {
  const c = checkSchedule(input);
  const now = Date.now();
  const row: ScheduleRow = {
    id: uuid(),
    name: c.name,
    timeZone: c.timeZone,
    team: c.team ?? null,
    tags: c.tags ?? [],
    layers: c.layers,
    maxConcurrent: c.maxConcurrent ?? null,
    overrides: [],
    createdAt: now,
    updatedAt: now,
  };
  db().schedules.push(row);
  return scheduleView(row);
}

export async function updateSchedule(id: string, input: OnCallScheduleInput): Promise<OnCallSchedule> {
  const row = findSchedule(id);
  const c = checkSchedule(input);
  Object.assign(row, { name: c.name, timeZone: c.timeZone, team: c.team ?? null, tags: c.tags ?? [], layers: c.layers, maxConcurrent: c.maxConcurrent ?? null, updatedAt: Date.now() });
  return scheduleView(row);
}

export async function deleteSchedule(id: string): Promise<void> {
  const s = db();
  findSchedule(id);
  s.schedules = s.schedules.filter((x) => x.id !== id);
}

function shifts(layers: OnCallLayer[], overrides: ScheduleRow['overrides'], maxConcurrent: number | null | undefined, tz: string, range: OnCallRange): OnCallScheduleShifts {
  const ovs = overrides.map((o) => ({ userId: o.userId, start: o.start, end: o.end }));
  const perLayer = layers.map((l) => ({ layerId: l.id, shifts: layerShifts(l, tz, range.from, range.to) }));
  const finalShifts = finalSchedule(layers, ovs, maxConcurrent, tz, range.from, range.to);
  const ids = [...new Set([...perLayer.flatMap((l) => l.shifts.map((s) => s.userId)), ...finalShifts.flatMap((s) => s.userIds)])].sort();
  return { layers: perLayer, finalShifts, overrides: overrides.filter((o) => o.end > range.from && o.start < range.to).map(overrideView), users: people(ids) };
}

export async function scheduleShifts(id: string, range: OnCallRange): Promise<OnCallScheduleShifts> {
  checkRange(range);
  const s = findSchedule(id);
  return shifts(s.layers, s.overrides, s.maxConcurrent, s.timeZone, range);
}

export async function previewSchedule(req: OnCallPreviewRequest): Promise<OnCallScheduleShifts> {
  checkRange(req.range);
  if (!isTimeZone(req.schedule.timeZone)) fail(`Unknown time zone ${req.schedule.timeZone}`);
  const layers = req.schedule.layers.filter((l) => !layerProblem(l));
  const overrides = req.scheduleId ? (db().schedules.find((s) => s.id === req.scheduleId)?.overrides ?? []) : [];
  return shifts(layers, overrides, req.schedule.maxConcurrent, req.schedule.timeZone, req.range);
}

export async function createOverride(id: string, req: CreateOnCallOverride): Promise<OnCallOverride> {
  if (req.end <= req.start || req.end - req.start > 366 * DAY) fail('An override ends after it starts and lasts at most a year');
  const s = findSchedule(id);
  if (!person(req.userId)) fail(`Unknown or inactive user ${req.userId}`);
  const o = { id: uuid(), userId: req.userId, start: req.start, end: req.end };
  s.overrides.push(o);
  return overrideView(o);
}

export async function deleteOverride(id: string, overrideId: string): Promise<void> {
  const s = findSchedule(id);
  if (!s.overrides.some((o) => o.id === overrideId)) fail('Override not found');
  s.overrides = s.overrides.filter((o) => o.id !== overrideId);
}

export async function myShifts(range: OnCallRange): Promise<OnCallMyShift[]> {
  checkRange(range);
  const out: OnCallMyShift[] = [];
  for (const s of db().schedules) {
    const fin = finalSchedule(s.layers, spans(s), s.maxConcurrent, s.timeZone, range.from, range.to);
    for (const [start, end] of shiftsFor(ME_ID, fin)) out.push({ scheduleId: s.id, scheduleName: s.name, start, end });
  }
  return out.sort((a, b) => a.start - b.start);
}

// ---------------------------------------------------------------- policies

function policyView(p: PolicyRow): OnCallPolicy {
  const usedBy = Object.entries(db().routing)
    .filter(([, pid]) => pid === p.id)
    .map(([team]) => team)
    .sort();
  return {
    id: p.id,
    name: p.name,
    team: p.team,
    tags: [...p.tags],
    steps: structuredClone(p.steps),
    repeat: p.repeat,
    resolveAfterMinutes: p.resolveAfterMinutes,
    usedBy,
    createdAt: p.createdAt,
    updatedAt: p.updatedAt,
  };
}

function findPolicy(id: string): PolicyRow {
  return db().policies.find((p) => p.id === id) ?? fail('Escalation policy not found');
}

function checkPolicy(input: OnCallPolicyInput): Omit<PolicyRow, 'id' | 'createdAt' | 'updatedAt'> {
  const name = text(input.name, 'The name', 200);
  const steps = input.steps;
  if (!steps.length || steps.length > 10) fail('An escalation policy has 1–10 steps');
  steps.forEach((s, i) => {
    if (!s.targets.length || s.targets.length > 20) fail(`Step ${i + 1}: notify 1–20 users, schedules or teams`);
    if (s.escalateAfterMinutes < 1 || s.escalateAfterMinutes > 1440) fail(`Step ${i + 1}: escalate after 1–1440 minutes`);
    for (const t of s.targets) {
      if (t.kind === 'user' && !person(t.id)) fail(`Unknown or inactive user ${t.id}`);
      if (t.kind === 'team') checkTeam(t.id);
      if (t.kind === 'schedule' && !db().schedules.some((x) => x.id === t.id)) fail(`Unknown schedule ${t.id}`);
    }
  });
  const repeat = input.repeat ?? 0;
  if (repeat < 0 || repeat > 10) fail('Repeat the policy 0–10 times');
  const resolve = input.resolveAfterMinutes ?? null;
  if (resolve != null && (resolve < 1 || resolve > 43_200)) fail('Resolve pages automatically after 1–43200 minutes');
  return { name, team: checkTeam(input.team), tags: tags(input.tags), steps: structuredClone(steps), repeat, resolveAfterMinutes: resolve };
}

export async function listPolicies(): Promise<OnCallPolicy[]> {
  return db()
    .policies.slice()
    .sort((a, b) => a.name.localeCompare(b.name))
    .map(policyView);
}

export async function getPolicy(id: string): Promise<OnCallPolicy> {
  return policyView(findPolicy(id));
}

export async function createPolicy(input: OnCallPolicyInput): Promise<OnCallPolicy> {
  const now = Date.now();
  const row: PolicyRow = { id: uuid(), ...checkPolicy(input), createdAt: now, updatedAt: now };
  db().policies.push(row);
  return policyView(row);
}

export async function updatePolicy(id: string, input: OnCallPolicyInput): Promise<OnCallPolicy> {
  const row = findPolicy(id);
  Object.assign(row, checkPolicy(input), { updatedAt: Date.now() });
  return policyView(row);
}

export async function deletePolicy(id: string): Promise<void> {
  const s = db();
  findPolicy(id);
  s.policies = s.policies.filter((p) => p.id !== id);
  for (const [team, pid] of Object.entries(s.routing)) if (pid === id) delete s.routing[team];
  for (const p of s.pages) if (p.policyId === id) p.policyId = null;
}

// ---------------------------------------------------------------- teams

export async function listTeams(): Promise<OnCallTeam[]> {
  const s = db();
  const now = Date.now();
  return mockTeamRoster()
    .slice()
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((t) => {
      const owned = s.schedules.filter((x) => x.team === t.handle);
      const pid = s.routing[t.handle];
      const policy = pid ? s.policies.find((p) => p.id === pid) : undefined;
      const onCall = [...new Set(owned.flatMap((x) => onCallAt(x.layers, spans(x), x.maxConcurrent, x.timeZone, now)))];
      return {
        handle: t.handle,
        name: t.name,
        memberCount: t.members.length,
        policy: policy ? { id: policy.id, name: policy.name } : null,
        schedules: owned.map((x) => ({ id: x.id, name: x.name })),
        onCallNow: people(onCall),
        openPages: s.pages.filter((p) => p.team === t.handle && p.status !== 'resolved').length,
        mine: t.members.includes(ME_ID),
      };
    });
}

export async function setRouting(handle: string, policyId: string | null): Promise<void> {
  const s = db();
  if (!mockTeamRoster().some((t) => t.handle === handle)) fail(`Team @${handle} not found`);
  if (!policyId) delete s.routing[handle];
  else {
    findPolicy(policyId);
    s.routing[handle] = policyId;
  }
}

// ---------------------------------------------------------------- pages

function ev(page: PageRow, kind: string, actor: string, body: string, at: number): void {
  page.timeline.push({ id: page.timeline.length + 1, kind, actor, body, createdAt: at });
}

function newPage(
  s: Db,
  req: { title: string; description: string; urgency: 'high' | 'low'; team: string | null; policyId: string | null },
  source: PageRow['source'],
  createdBy: string,
  at: number,
): PageRow {
  const policyId = req.policyId ?? (req.team ? (s.routing[req.team] ?? null) : null);
  const policy = policyId ? s.policies.find((p) => p.id === policyId) : undefined;
  const page: PageRow = {
    id: s.seq++,
    title: req.title,
    description: req.description,
    urgency: req.urgency,
    status: 'triggered',
    team: req.team,
    policyId: policy?.id ?? null,
    source,
    incidentId: null,
    monitorId: null,
    responders: [],
    createdBy,
    createdAt: at,
    acknowledgedBy: null,
    acknowledgedAt: null,
    resolvedBy: null,
    resolvedAt: null,
    step: 0,
    round: 0,
    nextEscalationAt: at,
    resolveAt: policy?.resolveAfterMinutes ? at + policy.resolveAfterMinutes * MIN : null,
    timeline: [],
  };
  const to = req.team && policy ? `Paged @${req.team} (${policy.name})` : req.team ? `Paged every member of @${req.team} (the team has no escalation policy)` : `Paged ${policy?.name ?? ''}`;
  ev(page, 'triggered', createdBy, to, at);
  s.pages.push(page);
  return page;
}

function stepsOf(s: Db, page: PageRow): { steps: OnCallStep[]; repeat: number } {
  const policy = page.policyId ? s.policies.find((p) => p.id === page.policyId) : undefined;
  if (policy) return { steps: policy.steps, repeat: policy.repeat };
  return { steps: [{ targets: [{ kind: 'team', id: page.team ?? '' }], escalateAfterMinutes: 1 }], repeat: 0 };
}

function targetsAt(s: Db, step: OnCallStep, at: number): string[] {
  const ids: string[] = [];
  for (const t of step.targets) {
    if (t.kind === 'user') ids.push(t.id);
    else if (t.kind === 'team') ids.push(...(mockTeamRoster().find((x) => x.handle === t.id)?.members ?? []));
    else {
      const sc = s.schedules.find((x) => x.id === t.id);
      if (sc) ids.push(...onCallAt(sc.layers, spans(sc), sc.maxConcurrent, sc.timeZone, at));
    }
  }
  return [...new Set(ids)].filter((id) => person(id));
}

function channelsFor(s: Db, user: string, urgency: string): string[] {
  const p = s.profiles[user];
  const list = p ? (urgency === 'low' ? p.lowUrgency : p.highUrgency) : [];
  return list.length ? list : ['email'];
}

/** Runs the escalation steps that are due by `until` (the server's worker does this every few seconds). */
function escalate(s: Db, page: PageRow, until: number): void {
  if (page.status !== 'resolved' && page.resolveAt != null && page.resolveAt <= until) {
    // Steps due before the auto-resolve still ran.
    escalate(s, page, page.resolveAt - 1);
    setStatus(page, 'resolved', 'ObserveCraft', page.resolveAt);
    return;
  }
  while (page.status === 'triggered' && page.nextEscalationAt != null && page.nextEscalationAt <= until) {
    const at = page.nextEscalationAt;
    const { steps, repeat } = stepsOf(s, page);
    const step = steps[page.step];
    if (!step) {
      page.nextEscalationAt = null;
      break;
    }
    if (page.step > 0 || page.round > 0)
      ev(page, 'escalated', 'ObserveCraft', page.step === 0 ? `Not acknowledged: starting over (repeat ${page.round})` : `Not acknowledged: escalated to step ${page.step + 1}`, at);
    const users = targetsAt(s, step, at);
    if (!users.length) ev(page, 'failed', 'ObserveCraft', `Step ${page.step + 1}: nobody to notify (no one on call or no members)`, at);
    for (const u of users) {
      if (!page.responders.includes(u)) page.responders.push(u);
      const via = channelsFor(s, u, page.urgency).map((c) => (c === 'email' ? 'email' : `@${c}`));
      ev(page, 'notified', 'ObserveCraft', `Notified ${person(u)?.name} by ${via.join(', ')}`, at);
    }
    const wait = Math.max(1, step.escalateAfterMinutes) * MIN;
    if (page.step + 1 < steps.length) page.step += 1;
    else if (page.round < repeat) {
      page.step = 0;
      page.round += 1;
    } else {
      page.step += 1;
      page.nextEscalationAt = null;
      break;
    }
    page.nextEscalationAt = at + wait;
  }
}

function setStatus(page: PageRow, next: 'acknowledged' | 'resolved', actor: string, at: number): void {
  if (page.status === 'resolved') fail('The page is resolved');
  if (next === 'acknowledged') {
    if (page.status === 'acknowledged') fail('The page is already acknowledged');
    Object.assign(page, { status: 'acknowledged', acknowledgedBy: actor, acknowledgedAt: at, nextEscalationAt: null });
    ev(page, 'acknowledged', actor, 'Acknowledged; escalation stopped', at);
  } else {
    Object.assign(page, { status: 'resolved', resolvedBy: actor, resolvedAt: at, nextEscalationAt: null, acknowledgedBy: page.acknowledgedBy ?? actor, acknowledgedAt: page.acknowledgedAt ?? at });
    ev(page, 'resolved', actor, 'Resolved', at);
  }
}

function catchUp(): Db {
  const s = db();
  const now = Date.now();
  for (const p of s.pages) escalate(s, p, now);
  return s;
}

function pageView(s: Db, p: PageRow): OnCallPage {
  const policy = p.policyId ? s.policies.find((x) => x.id === p.policyId) : undefined;
  return {
    id: p.id,
    title: p.title,
    description: p.description,
    urgency: p.urgency,
    status: p.status,
    team: p.team,
    policy: policy ? { id: policy.id, name: policy.name } : null,
    source: p.source,
    incidentId: p.incidentId,
    monitorId: p.monitorId,
    responders: people(p.responders),
    createdBy: p.createdBy,
    createdAt: p.createdAt,
    acknowledgedBy: p.acknowledgedBy,
    acknowledgedAt: p.acknowledgedAt,
    resolvedBy: p.resolvedBy,
    resolvedAt: p.resolvedAt,
    nextEscalationAt: p.status === 'triggered' ? p.nextEscalationAt : null,
  };
}

const STATUS_FILTERS: Record<string, (p: PageRow) => boolean> = {
  active: (p) => p.status !== 'resolved',
  triggered: (p) => p.status === 'triggered',
  acknowledged: (p) => p.status === 'acknowledged',
  resolved: (p) => p.status === 'resolved',
  all: () => true,
};

export async function searchPages(req: OnCallPageSearch): Promise<OnCallPagesResult> {
  const s = catchUp();
  const byStatus = STATUS_FILTERS[req.status] ?? fail('status is active, triggered, acknowledged, resolved or all');
  const words = (req.query ?? '').toLowerCase().split(/\s+/).filter(Boolean);
  const base = s.pages.filter(
    (p) =>
      (!req.teams?.length || (p.team != null && req.teams.includes(p.team))) &&
      (!req.mine || p.responders.includes(ME_ID)) &&
      words.every((w) => p.title.toLowerCase().includes(w)) &&
      (req.from == null || p.createdAt >= req.from) &&
      (req.to == null || p.createdAt <= req.to),
  );
  const count = (k: string) => base.filter(STATUS_FILTERS[k]).length;
  const pages = base
    .filter(byStatus)
    .sort((a, b) => b.createdAt - a.createdAt)
    .slice(0, Math.min(500, Math.max(1, req.limit ?? 200)))
    .map((p) => pageView(s, p));
  return { pages, counts: { active: count('active'), triggered: count('triggered'), acknowledged: count('acknowledged'), resolved: count('resolved'), all: base.length } };
}

function findPage(id: number): PageRow {
  return catchUp().pages.find((p) => p.id === id) ?? fail('Page not found');
}

export async function getPage(id: number): Promise<OnCallPageDetail> {
  const p = findPage(id);
  return { page: pageView(db(), p), timeline: p.timeline.map((e) => ({ ...e })), canRespond: true };
}

export async function createPage(req: CreateOnCallPage): Promise<OnCallPage> {
  const s = db();
  const team = checkTeam(req.team);
  const policyId = req.policyId || null;
  if (!team && !policyId) fail('Page a team or an escalation policy');
  if (policyId) findPolicy(policyId);
  const urgency = req.urgency ?? 'high';
  if (urgency !== 'high' && urgency !== 'low') fail('urgency is high or low');
  const p = newPage(
    s,
    { title: text(req.title, 'The title', 300), description: (req.description ?? '').trim(), urgency: urgency as 'high' | 'low', team, policyId },
    'manual',
    mockMe().name,
    Date.now(),
  );
  escalate(s, p, Date.now());
  return pageView(s, p);
}

export async function acknowledgePage(id: number): Promise<OnCallPage> {
  const p = findPage(id);
  setStatus(p, 'acknowledged', mockMe().name, Date.now());
  return pageView(db(), p);
}

export async function resolvePage(id: number): Promise<OnCallPage> {
  const p = findPage(id);
  setStatus(p, 'resolved', mockMe().name, Date.now());
  return pageView(db(), p);
}

export async function reassignPage(id: number, req: ReassignOnCallPage): Promise<OnCallPage> {
  const s = db();
  const p = findPage(id);
  const team = checkTeam(req.team);
  const policyId = req.policyId || null;
  if (!team && !policyId) fail('Choose a team or an escalation policy');
  if (p.status === 'resolved') fail('The page is resolved');
  const pid = policyId ?? (team ? (s.routing[team] ?? null) : null);
  const policy = pid ? findPolicy(pid) : undefined;
  const now = Date.now();
  Object.assign(p, {
    status: 'triggered',
    team,
    policyId: policy?.id ?? null,
    step: 0,
    round: 0,
    nextEscalationAt: now,
    acknowledgedBy: null,
    acknowledgedAt: null,
    resolveAt: policy?.resolveAfterMinutes ? now + policy.resolveAfterMinutes * MIN : null,
  });
  ev(p, 'reassigned', mockMe().name, `Reassigned to ${[team && `@${team}`, policy?.name].filter(Boolean).join(' · ')}`, now);
  escalate(s, p, now);
  return pageView(s, p);
}

/** Incidents mock: "Page a team" from an incident creates an On-Call page. */
export function mockPageFromIncident(team: string, title: string, incidentId: number, incidentTitle: string): number {
  const s = db();
  const p = newPage(s, { title, description: `Incident IR-${incidentId}: ${incidentTitle}`, urgency: 'high', team, policyId: null }, 'incident', mockMe().name, Date.now());
  p.incidentId = incidentId;
  escalate(s, p, Date.now());
  return p.id;
}

/** Incidents mock: the status of a linked page. */
export function mockOnCallPageState(id: number): Pick<OnCallPage, 'status' | 'acknowledgedBy' | 'acknowledgedAt' | 'resolvedAt'> | null {
  const p = catchUp().pages.find((x) => x.id === id);
  return p ? { status: p.status, acknowledgedBy: p.acknowledgedBy, acknowledgedAt: p.acknowledgedAt, resolvedAt: p.resolvedAt } : null;
}

export function mockSetOnCallPageStatus(id: number, status: 'acknowledged' | 'resolved'): void {
  setStatus(findPage(id), status, mockMe().name, Date.now());
}

// ---------------------------------------------------------------- profile

export async function getProfile(): Promise<OnCallProfile> {
  const p = db().profiles[ME_ID];
  return {
    email: mockPerson(ME_ID)?.email ?? '',
    highUrgency: p ? [...p.highUrgency] : ['email'],
    lowUrgency: p ? [...p.lowUrgency] : ['email'],
    configured: !!p,
    channels: [{ handle: 'email', kind: 'email' }, ...CHANNELS.map((handle) => ({ handle, kind: handle.split('-')[0] }))],
    emailEnabled: true,
  };
}

export async function updateProfile(input: OnCallProfileInput): Promise<OnCallProfile> {
  const options = ['email', ...CHANNELS];
  const clean = (list: string[]) => {
    for (const h of list) if (!options.includes(h)) fail(`Unknown channel ${h}`);
    const out = [...new Set(list)];
    if (!out.length) fail('Choose at least one way to be notified');
    return out;
  };
  db().profiles[ME_ID] = { highUrgency: clean(input.highUrgency), lowUrgency: clean(input.lowUrgency) };
  return getProfile();
}

export async function testProfile(): Promise<OnCallTestResult[]> {
  return channelsFor(db(), ME_ID, 'high').map((channel) => ({ channel, status: 'sent', error: null }));
}

// ---------------------------------------------------------------- analytics

export async function analytics(req: OnCallAnalyticsRequest): Promise<OnCallAnalytics> {
  checkRange(req.range, 92);
  const tz = req.timeZone || 'UTC';
  if (!isTimeZone(tz)) fail(`Unknown time zone ${tz}`);
  const s = catchUp();
  const pages = s.pages.filter((p) => p.createdAt >= req.range.from && p.createdAt < req.range.to && (!req.teams?.length || (p.team != null && req.teams.includes(p.team))));
  const avg = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);
  const tta = (ps: PageRow[]) => avg(ps.filter((p) => p.acknowledgedAt != null).map((p) => (p.acknowledgedAt! - p.createdAt) / 1000));
  const ttr = (ps: PageRow[]) => avg(ps.filter((p) => p.resolvedAt != null).map((p) => (p.resolvedAt! - p.createdAt) / 1000));
  const perDay = new Map<string, { day: number; team: string; count: number }>();
  const byHour = Array.from({ length: 168 }, () => 0);
  for (const p of pages) {
    const wall = utcToWall(tz, p.createdAt);
    const dayWall = wall - (wall % DAY);
    const day = p.createdAt - (wall - dayWall);
    const key = `${dayWall}|${p.team ?? ''}`;
    const row = perDay.get(key) ?? { day, team: p.team ?? '', count: 0 };
    row.count += 1;
    perDay.set(key, row);
    const weekday = (new Date(wall).getUTCDay() + 6) % 7;
    byHour[weekday * 24 + new Date(wall).getUTCHours()] += 1;
  }
  const teams = [...new Set(pages.map((p) => p.team ?? ''))]
    .map((team) => {
      const ps = pages.filter((p) => (p.team ?? '') === team);
      return { team, count: ps.length, mttaSeconds: tta(ps), mttrSeconds: ttr(ps) };
    })
    .sort((a, b) => b.count - a.count || a.team.localeCompare(b.team));
  const titles = new Map<string, { title: string; source: string; count: number }>();
  for (const p of pages) {
    const row = titles.get(p.title) ?? { title: p.title, source: p.source, count: 0 };
    row.count += 1;
    titles.set(p.title, row);
  }
  return {
    total: pages.length,
    acknowledged: pages.filter((p) => p.acknowledgedAt != null).length,
    resolved: pages.filter((p) => p.resolvedAt != null).length,
    mttaSeconds: tta(pages),
    mttrSeconds: ttr(pages),
    perDay: [...perDay.values()].sort((a, b) => a.day - b.day || a.team.localeCompare(b.team)),
    byHour,
    teams,
    topSources: [...titles.values()].sort((a, b) => b.count - a.count || a.title.localeCompare(b.title)).slice(0, 10),
  };
}

/** Test hook: start over with fresh demo data. */
export function resetOnCallMock(): void {
  state = null;
}
