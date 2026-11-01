// On-Call helpers shared by the pages and the in-browser mock: who is on call when (the same rules as
// the server's routes/on_call_schedule.rs), calendar and timeline layout, and the .ics export.
//
// Local wall-clock times are handled as "wall ms": the local date and time read as if they were UTC
// (Date.UTC(y, m, d, h, min)), so rotations add local hours and keep handoffs at the same local time
// across DST changes.
import type { OnCallFinalShift, OnCallLayer, OnCallRestriction, OnCallShift, OnCallWeekday } from './types';

export const HOUR = 3_600_000;
export const DAY = 24 * HOUR;
export const WEEKDAYS: OnCallWeekday[] = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];

export interface OverrideSpan {
  userId: string;
  start: number;
  end: number;
}

// ---------------------------------------------------------------- time zones

const formatters = new Map<string, Intl.DateTimeFormat>();

function formatter(tz: string): Intl.DateTimeFormat {
  let f = formatters.get(tz);
  if (!f) {
    f = new Intl.DateTimeFormat('en-US', { timeZone: tz, hourCycle: 'h23', year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit' });
    formatters.set(tz, f);
  }
  return f;
}

export function isTimeZone(tz: string): boolean {
  try {
    formatter(tz);
    return true;
  } catch {
    return false;
  }
}

/** The instant's wall-clock time in `tz`, as wall ms. */
export function utcToWall(tz: string, ms: number): number {
  const parts: Record<string, number> = {};
  for (const p of formatter(tz).formatToParts(new Date(ms))) if (p.type !== 'literal') parts[p.type] = Number(p.value);
  const ms2 = ((ms % 1000) + 1000) % 1000;
  return Date.UTC(parts.year, parts.month - 1, parts.day, parts.hour % 24, parts.minute, parts.second) + ms2;
}

/** UTC offset of `tz` at an instant (ms, local − UTC). */
export function offsetAt(tz: string, ms: number): number {
  return utcToWall(tz, ms) - ms;
}

/** The instant of a local wall-clock time: the earlier one when ambiguous, read with the offset before the gap when it doesn't exist. */
export function wallToUtc(tz: string, wall: number): number {
  const offsets = new Set([offsetAt(tz, wall - 14 * HOUR), offsetAt(tz, wall), offsetAt(tz, wall + 14 * HOUR)]);
  const valid = [...offsets].map((o) => wall - o).filter((t) => utcToWall(tz, t) === wall);
  if (valid.length) return Math.min(...valid);
  const beforeGuess = wall - 3 * HOUR;
  const before = offsetAt(tz, beforeGuess - offsetAt(tz, beforeGuess));
  return wall - before;
}

/** "YYYY-MM-DDTHH:MM" → wall ms (NaN when malformed). */
export function parseLocal(s: string): number {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(s.trim());
  if (!m) return NaN;
  const [y, mo, d, h, mi] = m.slice(1).map(Number);
  if (mo < 1 || mo > 12 || d < 1 || d > 31 || h > 23 || mi > 59) return NaN;
  return Date.UTC(y, mo - 1, d, h, mi);
}

export function formatLocal(wall: number): string {
  return new Date(wall).toISOString().slice(0, 16);
}

/** "HH:MM" → minutes after midnight (NaN when malformed). */
export function parseTime(s: string): number {
  const m = /^(\d{2}):(\d{2})$/.exec(s.trim());
  if (!m || Number(m[1]) > 23 || Number(m[2]) > 59) return NaN;
  return Number(m[1]) * 60 + Number(m[2]);
}

/** 0 = Monday. */
function weekdayOfWall(wall: number): number {
  return (new Date(wall).getUTCDay() + 6) % 7;
}

// ---------------------------------------------------------------- schedules

/** Why a layer can't be saved, or null. */
export function layerProblem(l: OnCallLayer): string | null {
  if (!(l.rotationHours >= 1 && l.rotationHours <= 8760)) return `${l.name}: shifts last 1–8760 hours`;
  const from = parseLocal(l.effectiveFrom);
  if (Number.isNaN(from)) return `${l.name}: choose a start date and handoff time`;
  if (l.effectiveUntil) {
    const until = parseLocal(l.effectiveUntil);
    if (Number.isNaN(until)) return `${l.name}: the end is not a valid date`;
    if (until <= from) return `${l.name}: the end must come after the start`;
  }
  for (const r of l.restrictions ?? []) {
    const s = parseTime(r.startTime);
    const e = parseTime(r.endTime);
    if (Number.isNaN(s) || Number.isNaN(e)) return `${l.name}: restriction times are HH:MM`;
    if (r.startDay === r.endDay && s === e) return `${l.name}: a restriction can't start and end at the same time`;
  }
  return null;
}

function merge(spans: [number, number][]): [number, number][] {
  spans.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const out: [number, number][] = [];
  for (const [s, e] of spans) {
    const last = out[out.length - 1];
    if (last && s <= last[1]) last[1] = Math.max(last[1], e);
    else out.push([s, e]);
  }
  return out;
}

function restrictionWindows(tz: string, restrictions: OnCallRestriction[], from: number, to: number): [number, number][] {
  const firstWall = utcToWall(tz, from);
  const firstDay = Date.UTC(new Date(firstWall).getUTCFullYear(), new Date(firstWall).getUTCMonth(), new Date(firstWall).getUTCDate());
  let week = firstDay - (weekdayOfWall(firstDay) + 7) * DAY;
  const out: [number, number][] = [];
  while (wallToUtc(tz, week) < to) {
    for (const r of restrictions) {
      const st = parseTime(r.startTime);
      const et = parseTime(r.endTime);
      if (Number.isNaN(st) || Number.isNaN(et)) continue;
      const startWall = week + WEEKDAYS.indexOf(r.startDay) * DAY + st * 60_000;
      let endWall = week + WEEKDAYS.indexOf(r.endDay) * DAY + et * 60_000;
      if (endWall <= startWall) endWall += 7 * DAY;
      const s = wallToUtc(tz, startWall);
      const e = wallToUtc(tz, endWall);
      if (e > from && s < to) out.push([Math.max(s, from), Math.min(e, to)]);
    }
    week += 7 * DAY;
  }
  return merge(out);
}

/** The shifts of one layer overlapping [from, to), clipped to it. */
export function layerShifts(layer: OnCallLayer, tz: string, from: number, to: number): OnCallShift[] {
  const start = parseLocal(layer.effectiveFrom);
  if (Number.isNaN(start) || !layer.members.length || layer.rotationHours <= 0 || from >= to) return [];
  const until = layer.effectiveUntil ? parseLocal(layer.effectiveUntil) : NaN;
  const end = Math.min(to, Number.isNaN(until) ? Infinity : wallToUtc(tz, until));
  const len = layer.rotationHours * HOUR;
  const boundary = (k: number) => wallToUtc(tz, start + k * len);
  const elapsedH = Math.floor((utcToWall(tz, from) - start) / HOUR);
  let k = Math.max(0, Math.floor(elapsedH / layer.rotationHours) - 1);
  const windows = layer.restrictions?.length ? restrictionWindows(tz, layer.restrictions, from, end) : null;
  const out: OnCallShift[] = [];
  const n = layer.members.length;
  for (;;) {
    const s = boundary(k);
    if (s >= end) break;
    const e = boundary(k + 1);
    const userId = layer.members[((k % n) + n) % n];
    const a = Math.max(s, from);
    const b = Math.min(e, end);
    if (b > a) {
      if (!windows) out.push({ userId, start: a, end: b });
      else
        for (const [ws, we] of windows) {
          const x = Math.max(a, ws);
          const y = Math.min(b, we);
          if (y > x) out.push({ userId, start: x, end: y });
        }
    }
    k += 1;
  }
  return out;
}

/** The final schedule over [from, to): later layers first, cut to maxConcurrent; overrides replace it. */
export function finalSchedule(layers: OnCallLayer[], overrides: OverrideSpan[], maxConcurrent: number | null | undefined, tz: string, from: number, to: number): OnCallFinalShift[] {
  const perLayer = layers.map((l) => layerShifts(l, tz, from, to));
  const ovs = overrides.filter((o) => o.end > from && o.start < to && o.end > o.start);
  const points = new Set<number>([from, to]);
  for (const s of perLayer.flat()) points.add(s.start).add(s.end);
  for (const o of ovs) points.add(Math.max(o.start, from)).add(Math.min(o.end, to));
  const sorted = [...points].sort((a, b) => a - b);
  const cursor = perLayer.map(() => 0);
  const out: OnCallFinalShift[] = [];
  for (let i = 0; i + 1 < sorted.length; i++) {
    const a = sorted[i];
    const b = sorted[i + 1];
    const users: string[] = [];
    const covering = ovs.filter((o) => o.start <= a && o.end >= b);
    const isOverride = covering.length > 0;
    for (let li = perLayer.length - 1; li >= 0; li--) {
      const shifts = perLayer[li];
      while (cursor[li] < shifts.length && shifts[cursor[li]].end <= a) cursor[li] += 1;
      const s = shifts[cursor[li]];
      if (!isOverride && s && s.start <= a && s.end >= b && !users.includes(s.userId)) users.push(s.userId);
    }
    if (isOverride) {
      for (const o of covering) if (!users.includes(o.userId)) users.push(o.userId);
    } else if (maxConcurrent) users.splice(Math.max(1, maxConcurrent));
    if (!users.length) continue;
    const last = out[out.length - 1];
    if (last && last.end === a && last.isOverride === isOverride && last.userIds.join() === users.join()) last.end = b;
    else out.push({ userIds: users, start: a, end: b, isOverride });
  }
  return out;
}

export function onCallAt(layers: OnCallLayer[], overrides: OverrideSpan[], maxConcurrent: number | null | undefined, tz: string, at: number): string[] {
  return finalSchedule(layers, overrides, maxConcurrent, tz, at, at + 1)[0]?.userIds ?? [];
}

/** Stretches of a final schedule that include `user`, joined when contiguous. */
export function shiftsFor(user: string, fin: OnCallFinalShift[]): [number, number][] {
  const out: [number, number][] = [];
  for (const s of fin) {
    if (!s.userIds.includes(user)) continue;
    const last = out[out.length - 1];
    if (last && last[1] === s.start) last[1] = s.end;
    else out.push([s.start, s.end]);
  }
  return out;
}

export function rotationLabel(hours: number): string {
  if (hours === 24) return 'Daily';
  if (hours === 168) return 'Weekly';
  if (hours === 336) return 'Every 2 weeks';
  if (hours % 24 === 0) return `Every ${hours / 24} days`;
  return `Every ${hours} hours`;
}

// ---------------------------------------------------------------- calendar and timeline

export interface CalendarDay {
  /** Midnight of the day, wall ms. */
  wall: number;
  day: number;
  inMonth: boolean;
}

/** The weeks (Sunday → Saturday) covering a month (month 0–11). */
export function monthGrid(year: number, month: number): CalendarDay[][] {
  const first = Date.UTC(year, month, 1);
  const start = first - new Date(first).getUTCDay() * DAY;
  const last = Date.UTC(year, month + 1, 0);
  const weeks: CalendarDay[][] = [];
  for (let w = start; w <= last; w += 7 * DAY) {
    weeks.push(Array.from({ length: 7 }, (_, i) => ({ wall: w + i * DAY, day: new Date(w + i * DAY).getUTCDate(), inMonth: new Date(w + i * DAY).getUTCMonth() === month })));
  }
  return weeks;
}

/** Where a span sits on a timeline [from, to): percentages, or null when outside. */
export function timelineBlock(start: number, end: number, from: number, to: number): { left: number; width: number } | null {
  const a = Math.max(start, from);
  const b = Math.min(end, to);
  if (b <= a || to <= from) return null;
  return { left: ((a - from) / (to - from)) * 100, width: ((b - a) / (to - from)) * 100 };
}

/** The day columns of a timeline: midnight of each local day in [from, to) as instants. */
export function dayColumns(tz: string, from: number, to: number): number[] {
  const w = utcToWall(tz, from);
  let day = Date.UTC(new Date(w).getUTCFullYear(), new Date(w).getUTCMonth(), new Date(w).getUTCDate());
  const out: number[] = [];
  for (let t = wallToUtc(tz, day); t < to; day += DAY, t = wallToUtc(tz, day)) if (t >= from) out.push(t);
  return out;
}

/** A stable palette slot (1–8) per person, for shift colors. */
export function colorSlot(userId: string): number {
  let h = 0;
  for (let i = 0; i < userId.length; i++) h = (Math.imul(h, 31) + userId.charCodeAt(i)) >>> 0;
  return (h % 8) + 1;
}

// ---------------------------------------------------------------- export

function icsDate(ms: number): string {
  return new Date(ms)
    .toISOString()
    .replace(/[-:]/g, '')
    .replace(/\.\d{3}/, '');
}

function icsText(s: string): string {
  return s.replace(/\\/g, '\\\\').replace(/;/g, '\\;').replace(/,/g, '\\,').replace(/\n/g, '\\n');
}

/** An iCalendar file of shifts (UTC times, CRLF line ends). */
export function shiftsToIcs(shifts: { scheduleId: string; scheduleName: string; start: number; end: number }[], now: number, host = 'observecraft'): string {
  const lines = ['BEGIN:VCALENDAR', 'VERSION:2.0', 'PRODID:-//ObserveCraft//On-Call//EN', 'CALSCALE:GREGORIAN', 'METHOD:PUBLISH'];
  for (const s of shifts) {
    lines.push(
      'BEGIN:VEVENT',
      `UID:${s.scheduleId}-${s.start}@${host}`,
      `DTSTAMP:${icsDate(now)}`,
      `DTSTART:${icsDate(s.start)}`,
      `DTEND:${icsDate(s.end)}`,
      `SUMMARY:${icsText(`On call: ${s.scheduleName}`)}`,
      'END:VEVENT',
    );
  }
  lines.push('END:VCALENDAR');
  return lines.join('\r\n') + '\r\n';
}

// ---------------------------------------------------------------- formatting

export function fmtDuration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds)) return '—';
  if (seconds < 60) return `${Math.round(seconds)}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  if (seconds < 86_400) return `${(seconds / 3600).toFixed(1).replace(/\.0$/, '')}h`;
  return `${(seconds / 86_400).toFixed(1).replace(/\.0$/, '')}d`;
}

/** A default schedule name: "<first name>'s Schedule – Oct 28, 2026". */
export function defaultScheduleName(userName: string, now: number): string {
  const first = userName.split(' ')[0] || userName;
  const date = new Date(now).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
  return `${first}'s Schedule – ${date}`;
}

// ---------------------------------------------------------------- lists and forms

/** Teams set up for On-Call: routed to an escalation policy or owning a schedule. */
export function onboardedTeams<T extends { policy?: unknown; schedules: unknown[] }>(teams: T[]): T[] {
  return teams.filter((t) => t.policy != null || t.schedules.length > 0);
}

function minutesLabel(m: number): string {
  if (m % 1440 === 0) return m === 1440 ? '1 day' : `${m / 1440} days`;
  if (m % 60 === 0) return m === 60 ? '1 hour' : `${m / 60} hours`;
  return m === 1 ? '1 minute' : `${m} minutes`;
}

/** Dropdown choices for a duration in minutes; a current value outside the list stays selectable. */
export function minuteChoices(choices: number[], current: number): { value: string; label: string }[] {
  const all = choices.includes(current) ? choices : [...choices, current].sort((a, b) => a - b);
  return all.map((m) => ({ value: String(m), label: minutesLabel(m) }));
}

/** Why an escalation policy can't be saved yet (the server's limits), or null. */
export function policyProblem(p: { name: string; steps: { targets: unknown[]; escalateAfterMinutes: number }[]; repeat?: number | null }): string | null {
  if (!p.name.trim()) return 'Name the escalation policy';
  if (!p.steps.length) return 'Add at least one step';
  for (const [i, s] of p.steps.entries()) {
    if (!s.targets.length) return `Add a recipient to step ${i + 1}`;
    if (!(s.escalateAfterMinutes >= 1)) return `Step ${i + 1} must escalate after at least 1 minute`;
  }
  if ((p.repeat ?? 0) > 10) return 'Repeat at most 10 times';
  return null;
}

/** A time range shortened to its last `maxDays` days. */
export function clampRange(r: { from: number; to: number }, maxDays: number): { from: number; to: number } {
  return { from: Math.max(r.from, r.to - maxDays * DAY), to: r.to };
}
