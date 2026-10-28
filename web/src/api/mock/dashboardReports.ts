// In-browser dashboard reports for mock mode. State lives in memory for the session. Validation,
// schedule descriptions and the rendered message follow the server
// (server/crates/oc-api/src/routes/dashboard_reports.rs). Email is "not configured" here, so sending
// reaches only the mock's channels (nothing leaves the browser).
import type { DashboardReport, DashboardReportDelivery, DashboardReportInput, DashboardReportOptions, DashboardReportPreview, ReportSchedule, ReportWidgetSummary } from '../dashboardReports/types';
import { TIME_FRAMES, describeSchedule } from '../dashboardReports/types';
import type { Widget } from '../dashboards/types';
import { resolveQuery } from '../../lib/metricQuery';
import * as dashboards from './dashboards';
import { queryTimeseries } from './metricsEngine';
import { mockMe, mockUserNamed } from './identity';

const MAX_RECIPIENTS = 50;
const MAX_MESSAGE_CHARS = 5_000;
const MAX_SERIES_PER_QUERY = 5;
const CHANNELS = ['slack-checkout-alerts', 'slack-billing', 'webhook-ops-digest'];
const EMAIL_OFF = "Email isn't configured on this install (OC_SMTP_URL)";
const ORIGIN = typeof window !== 'undefined' ? window.location.origin : 'http://localhost:5173';

const fail = (message: string): never => {
  throw new Error(message);
};
const copy = <T>(v: T): T => structuredClone(v);

// ================================================================ schedule

/** Wall-clock parts of `ms` in `tz`. */
function parts(ms: number, tz: string) {
  const f = new Intl.DateTimeFormat('en-US', { timeZone: tz, year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', hourCycle: 'h23' });
  const p = Object.fromEntries(f.formatToParts(new Date(ms)).map((x) => [x.type, x.value]));
  return { y: Number(p.year), m: Number(p.month), d: Number(p.day), h: Number(p.hour), mi: Number(p.minute) };
}

/** UTC ms of a wall-clock time in `tz` (two passes settle the zone offset). */
function fromLocal(y: number, m: number, d: number, h: number, mi: number, tz: string): number {
  const wall = Date.UTC(y, m - 1, d, h, mi);
  let guess = wall;
  for (let i = 0; i < 2; i++) {
    const p = parts(guess, tz);
    guess += wall - Date.UTC(p.y, p.m - 1, p.d, p.h, p.mi);
  }
  return guess;
}

function checkZone(tz: string): string {
  const z = tz.trim() || 'UTC';
  try {
    new Intl.DateTimeFormat('en-US', { timeZone: z });
  } catch {
    fail(`${z} isn't a known time zone (use an IANA name such as UTC or Europe/Paris)`);
  }
  return z;
}

/** The first send strictly after `after` (as the server's cron-based schedule). */
export function nextRun(s: ReportSchedule, after: number): number {
  const m = /^(\d\d):(\d\d)$/.exec(s.time);
  if (!m || Number(m[1]) > 23 || Number(m[2]) > 59) fail(`${s.time} isn't a time (HH:MM)`);
  if (s.frequency === 'weekly' && (s.weekday == null || s.weekday < 0 || s.weekday > 6)) fail('A weekly report needs a weekday (0 = Sunday … 6 = Saturday)');
  if (s.frequency === 'monthly' && (s.monthDay == null || s.monthDay < 1 || s.monthDay > 28)) fail('A monthly report needs a day of the month from 1 to 28');
  if (!['daily', 'weekly', 'monthly'].includes(s.frequency)) fail(`Unknown frequency ${s.frequency} (daily, weekly or monthly)`);
  const tz = checkZone(s.timezone);
  const start = parts(after, tz);
  for (let offset = 0; offset < 62; offset++) {
    const day = new Date(Date.UTC(start.y, start.m - 1, start.d + offset));
    if (s.frequency === 'weekly' && day.getUTCDay() !== s.weekday) continue;
    if (s.frequency === 'monthly' && day.getUTCDate() !== s.monthDay) continue;
    const at = fromLocal(day.getUTCFullYear(), day.getUTCMonth() + 1, day.getUTCDate(), Number(m![1]), Number(m![2]), tz);
    if (at > after) return at;
  }
  return fail('The schedule never fires');
}

// ================================================================ validation (as the server)

function validate(i: DashboardReportInput): string[] {
  const name = i.name.trim();
  if (!name || [...name].length > 200) fail('The report name must be 1–200 characters');
  if (!TIME_FRAMES.some((t) => t.key === i.timeFrame)) fail(`Unknown time frame ${i.timeFrame} (use one of ${TIME_FRAMES.map((t) => t.key).join(', ')})`);
  nextRun(i.schedule, 0);
  if ([...(i.message ?? '')].length > MAX_MESSAGE_CHARS) fail(`The message is at most ${MAX_MESSAGE_CHARS} characters`);
  const out: string[] = [];
  for (const raw of i.recipients ?? []) {
    const r = raw.trim().replace(/^@/, '').toLowerCase();
    if (!r) continue;
    const ok = r.includes('@') ? /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(r) && r.length <= 254 : /^(slack|webhook)-[a-z0-9_-]+$/.test(r) && r.length <= 80;
    if (!ok) fail(`${raw} isn't an email address or a channel handle (@slack-…, @webhook-…)`);
    if (!out.includes(r)) out.push(r);
  }
  if (out.length > MAX_RECIPIENTS) fail(`A report has at most ${MAX_RECIPIENTS} recipients`);
  return out;
}

// ================================================================ store

interface Row extends Omit<DashboardReport, 'subscribed' | 'subscriberCount' | 'scheduleLabel'> {
  subscribers: string[];
}

let store: Row[] | null = null;
let seq = 0;
const newId = () => `rp-${(++seq).toString(36).padStart(3, '0')}-${Date.now().toString(36).slice(-4)}`;

async function dashboardTitle(id: string): Promise<string> {
  const page = await dashboards.searchDashboards({ query: '', preset: 'all', listId: null, team: null, sort: { field: 'title', dir: 'asc' }, offset: 0, limit: 10_000 });
  return page.dashboards.find((d) => d.id === id)?.title ?? fail(`Dashboard ${id} not found`);
}

async function db(): Promise<Row[]> {
  if (store) return store;
  const page = await dashboards.searchDashboards({ query: '', preset: 'custom', listId: null, team: null, sort: { field: 'title', dir: 'asc' }, offset: 0, limit: 10_000 });
  const byTitle = (t: string) => page.dashboards.find((d) => d.title === t);
  const now = Date.now();
  const me = mockMe();
  const priya = mockUserNamed('Priya Shah') ?? me;
  const rows: Row[] = [];
  const add = (dash: { id: string; title: string } | undefined, r: Omit<Row, 'id' | 'dashboardId' | 'dashboardTitle' | 'createdAt' | 'updatedAt' | 'nextRunAt'>) => {
    if (!dash) return;
    rows.push({
      ...r,
      id: `rp-seed-${rows.length + 1}`,
      dashboardId: dash.id,
      dashboardTitle: dash.title,
      createdAt: now - 20 * 86_400_000,
      updatedAt: now - 3 * 86_400_000,
      nextRunAt: nextRun(r.schedule, now),
    });
  };
  add(byTitle('Billing'), {
    name: 'Billing weekly summary',
    templateValues: { env: 'prod' },
    timeFrame: '1w',
    schedule: { frequency: 'weekly', time: '09:00', weekday: 1, monthDay: null, timezone: 'Europe/Paris' },
    recipients: ['billing-team@example.com', 'slack-billing'],
    message: 'Numbers for the Monday billing review.',
    author: me,
    subscribers: [],
    lastSentAt: now - 2 * 86_400_000,
    lastStatus: 'partial',
    lastError: `billing-team@example.com: ${EMAIL_OFF}`,
  });
  add(byTitle('Service Overview — Production'), {
    name: 'Daily production health',
    templateValues: { env: 'prod', service: '*' },
    timeFrame: '1d',
    schedule: { frequency: 'daily', time: '08:00', weekday: null, monthDay: null, timezone: 'UTC' },
    recipients: ['slack-checkout-alerts'],
    message: '',
    author: priya,
    subscribers: [me.id],
    lastSentAt: now - 6 * 3_600_000,
    lastStatus: 'sent',
    lastError: null,
  });
  store = rows;
  return rows;
}

const find = async (id: string) => (await db()).find((r) => r.id === id) ?? fail(`Report ${id} not found`);

function view(r: Row): DashboardReport {
  const { subscribers, ...rest } = r;
  return copy({ ...rest, scheduleLabel: describeSchedule(r.schedule), subscribed: subscribers.includes(mockMe().id), subscriberCount: subscribers.length });
}

// ================================================================ endpoints

export async function reportOptions(): Promise<DashboardReportOptions> {
  return { emailEnabled: false, channels: [...CHANNELS], timeFrames: TIME_FRAMES.map(({ key, label }) => ({ key, label })) };
}

export async function listReports(): Promise<DashboardReport[]> {
  return (await db()).map(view).sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
}

export async function createReport(i: DashboardReportInput): Promise<DashboardReport> {
  const recipients = validate(i);
  const title = await dashboardTitle(i.dashboardId);
  const now = Date.now();
  const r: Row = {
    id: newId(),
    name: i.name.trim(),
    dashboardId: i.dashboardId,
    dashboardTitle: title,
    templateValues: { ...(i.templateValues ?? {}) },
    timeFrame: i.timeFrame,
    schedule: copy(i.schedule),
    recipients,
    message: (i.message ?? '').trim(),
    author: mockMe(),
    subscribers: [],
    nextRunAt: nextRun(i.schedule, now),
    lastSentAt: null,
    lastStatus: null,
    lastError: null,
    createdAt: now,
    updatedAt: now,
  };
  (await db()).push(r);
  return view(r);
}

export async function updateReport(id: string, i: DashboardReportInput): Promise<DashboardReport> {
  const r = await find(id);
  const recipients = validate(i);
  const title = await dashboardTitle(i.dashboardId);
  Object.assign(r, {
    name: i.name.trim(),
    dashboardId: i.dashboardId,
    dashboardTitle: title,
    templateValues: { ...(i.templateValues ?? {}) },
    timeFrame: i.timeFrame,
    schedule: copy(i.schedule),
    recipients,
    message: (i.message ?? '').trim(),
    nextRunAt: nextRun(i.schedule, Date.now()),
    updatedAt: Date.now(),
  });
  return view(r);
}

export async function deleteReport(id: string): Promise<void> {
  await find(id);
  store = (await db()).filter((r) => r.id !== id);
}

export async function setReportSubscription(id: string, subscribed: boolean): Promise<DashboardReport> {
  const r = await find(id);
  const me = mockMe().id;
  r.subscribers = r.subscribers.filter((s) => s !== me);
  if (subscribed) r.subscribers.push(me);
  return view(r);
}

// ================================================================ summary

const fmt = (v: number | null | undefined) => {
  if (v == null) return '–';
  if (Math.abs(v) >= 1e6) return v.toExponential(3).replace('e+', 'e');
  if (v !== 0 && Math.abs(v) < 0.01) return String(Math.round(v * 1e6) / 1e6);
  return String(Math.round(v * 100) / 100);
};

function flatten(widgets: Widget[]): Widget[] {
  return widgets.flatMap((w) => (w.definition.type === 'group' ? flatten(w.definition.widgets) : [w]));
}

const KIND_TITLES: Record<string, string> = {
  timeseries: 'Timeseries',
  query_value: 'Query value',
  note: 'Note',
  toplist: 'Top list',
  table: 'Table',
  log_stream: 'Log stream',
  monitor_summary: 'Monitor summary',
  image: 'Image',
  heatmap: 'Heatmap',
};
const kindTitle = (k: string) => KIND_TITLES[k] ?? k.replace(/_/g, ' ');

export async function previewReport(id: string): Promise<DashboardReportPreview> {
  const r = await find(id);
  const d = await dashboards.getDashboard(r.dashboardId);
  const frame = TIME_FRAMES.find((t) => t.key === r.timeFrame) ?? TIME_FRAMES[2];
  const now = Date.now();
  const range = { from: now - frame.ms, to: now };
  const widgets: ReportWidgetSummary[] = [];
  for (const w of flatten(d.widgets)) {
    const def = w.definition;
    const kind = def.type === 'integration' ? String(def.original.type ?? 'widget') : def.type;
    const title = def.title?.trim() || (def.type === 'integration' ? String(def.original.title ?? '') : '') || kindTitle(kind);
    const queries = def.type === 'timeseries' ? def.requests.map((q) => q.query) : def.type === 'query_value' ? [def.request.query] : [];
    const summary: ReportWidgetSummary = { title, kind, series: [], error: null };
    for (const q of queries) {
      const resolved = resolveQuery(q, d.templateVariables, r.templateValues);
      try {
        const series = await queryTimeseries({ queries: [resolved], range, points: 120 });
        for (const s of series.slice(0, MAX_SERIES_PER_QUERY)) {
          const vals = s.points.map((p) => p[1]).filter(Number.isFinite);
          summary.series.push({ label: s.name, current: vals.at(-1) ?? null, min: vals.length ? Math.min(...vals) : null, max: vals.length ? Math.max(...vals) : null });
        }
      } catch (e) {
        summary.error = (e as Error).message;
      }
    }
    widgets.push(summary);
  }
  const vars = Object.entries(r.templateValues)
    .filter(([, v]) => v !== '*')
    .map(([k, v]) => `&tpl_var_${encodeURIComponent(k)}=${encodeURIComponent(v)}`)
    .join('');
  const link = `${ORIGIN}/dashboards/${r.dashboardId}?time=${r.timeFrame}${vars}`;
  const lines = widgets.flatMap((w) =>
    w.error
      ? [`• ${w.title} (${kindTitle(w.kind).toLowerCase()}): couldn't be summarized: ${w.error}`]
      : w.series.length === 0
        ? [`• ${w.title} (${kindTitle(w.kind).toLowerCase()}): see the dashboard`]
        : [`• ${w.title}`, ...w.series.map((s) => `    ${s.label}: current ${fmt(s.current)} · min ${fmt(s.min)} · max ${fmt(s.max)}`)],
  );
  const intro = r.message.trim() ? `${r.message.trim()}\n\n` : '';
  const body = `${r.name}\n\nDashboard: ${r.dashboardTitle}\nTime frame: ${frame.label}\n\n${intro}${lines.length ? lines.join('\n') : 'This dashboard has no widgets.'}\n\nOpen the dashboard: ${link}\n`;
  return { subject: `[Report] ${r.dashboardTitle} · ${frame.label}`, body, widgets, range };
}

export async function sendReport(id: string): Promise<DashboardReportDelivery[]> {
  const r = await find(id);
  const me = mockMe();
  const recipients = [...r.recipients];
  if (r.subscribers.includes(me.id)) {
    const email = `${me.name.toLowerCase().replace(/\s+/g, '.')}@example.com`;
    if (!recipients.includes(email)) recipients.push(email);
  }
  const out: DashboardReportDelivery[] = recipients.map((to) =>
    to.includes('@')
      ? { recipient: to, status: 'not_sent', error: EMAIL_OFF }
      : CHANNELS.includes(to)
        ? { recipient: to, status: 'sent', error: null }
        : { recipient: to, status: 'not_sent', error: `No channel named @${to}. Add it under Monitors → Settings → Notification Channels.` },
  );
  const sent = out.filter((d) => d.status === 'sent').length;
  r.lastSentAt = Date.now();
  r.lastStatus = out.length && sent === out.length ? 'sent' : sent > 0 ? 'partial' : 'failed';
  const errors = out.filter((d) => d.error).map((d) => `${d.recipient}: ${d.error}`);
  r.lastError = out.length === 0 ? 'The report has no recipients' : errors.length ? errors.join('; ') : null;
  return copy(out);
}
