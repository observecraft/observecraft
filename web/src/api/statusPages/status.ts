// Status page helpers shared by the pages, the public rendering and the in-browser mock: current and
// overall statuses (as routes/status_pages.rs computes them), slugs, groups and the uptime bars.
import type { ComponentStatus, NoticeImpact, NoticeUpdate, StatusPageComponent } from './types';

const RANK: Record<ComponentStatus, number> = { operational: 0, degraded_performance: 1, partial_outage: 2, major_outage: 3 };
const DAY = 86_400_000;
export const UPTIME_DAYS = 60;
export const MAX_SLUG_CHARS = 64;

/** The worst status of the list (operational for none). */
export function worstStatus(statuses: Iterable<ComponentStatus>): ComponentStatus {
  let worst: ComponentStatus = 'operational';
  for (const s of statuses) if (RANK[s] > RANK[worst]) worst = s;
  return worst;
}

/** Components with their current status: the worst of the one set and the impacts of the open notices. */
export function currentComponents(components: Omit<StatusPageComponent, 'currentStatus'>[], notices: { resolvedAt?: number | null; impacts: NoticeImpact[] }[]): StatusPageComponent[] {
  const open = notices.filter((n) => n.resolvedAt == null).flatMap((n) => n.impacts);
  return components.map((c) => ({ ...c, currentStatus: worstStatus([c.status, ...open.filter((i) => i.componentId === c.id).map((i) => i.impact)]) }));
}

export const overallStatus = (components: { currentStatus: ComponentStatus }[]) => worstStatus(components.map((c) => c.currentStatus));

/** Null for a valid slug, else why it isn't. */
export function checkSlug(slug: string): string | null {
  return /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(slug) && slug.length <= MAX_SLUG_CHARS
    ? null
    : `The slug must be 1–${MAX_SLUG_CHARS} characters: lowercase letters, digits and dashes, not starting or ending with a dash`;
}

/** Null for no logo or an http(s) URL, else why it isn't valid. */
export function checkLogoUrl(url: string): string | null {
  const u = url.trim();
  if (!u) return null;
  if (u.length > 2048) return 'The logo URL is at most 2048 characters';
  try {
    if (['http:', 'https:'].includes(new URL(u).protocol)) return null;
  } catch {
    /* not a URL */
  }
  return 'The logo must be an http or https URL';
}

/** A slug suggested by a page name: `Storefront Status` → `storefront-status`. */
export function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, MAX_SLUG_CHARS)
    .replace(/-+$/, '');
}

/** Components by group, in the order each group first appears; ungrouped components form a group of their own. */
export function groupComponents<C extends { group?: string | null }>(components: C[]): { group: string | null; components: C[] }[] {
  const groups: { group: string | null; components: C[] }[] = [];
  for (const c of components) {
    const group = c.group || null;
    const g = groups.find((x) => x.group === group);
    if (g) g.components.push(c);
    else groups.push({ group, components: [c] });
  }
  return groups;
}

export interface UptimeDay {
  /** Midnight UTC. */
  start: number;
  /** The worst impact on the component that day. */
  status: ComponentStatus;
  /** Time under a partial or major outage that day. */
  downMs: number;
  /** The day ended before the component existed. */
  noData: boolean;
}

interface NoticeHistory {
  startedAt: number;
  resolvedAt?: number | null;
  impacts: NoticeImpact[];
  updates: Pick<NoticeUpdate, 'status' | 'impacts' | 'createdAt'>[];
}

/** The component's impacts over time from one notice: each update's impacts last until the next update. */
function segments(componentId: string, n: NoticeHistory, now: number): { from: number; to: number; impact: ComponentStatus }[] {
  const end = n.resolvedAt ?? now;
  const updates = [...n.updates].sort((a, b) => a.createdAt - b.createdAt);
  const steps = updates.length ? updates : [{ status: 'investigating' as const, impacts: n.impacts, createdAt: n.startedAt }];
  const out: { from: number; to: number; impact: ComponentStatus }[] = [];
  steps.forEach((u, i) => {
    if (u.status === 'resolved') return;
    const impact = u.impacts.find((x) => x.componentId === componentId)?.impact;
    const to = Math.min(steps[i + 1]?.createdAt ?? end, end);
    if (impact && impact !== 'operational' && to > u.createdAt) out.push({ from: u.createdAt, to, impact });
  });
  return out;
}

/** Total length of the union of the intervals. */
function unionLength(intervals: [number, number][]): number {
  let total = 0;
  let reach = -Infinity;
  for (const [from, to] of [...intervals].sort((a, b) => a[0] - b[0])) {
    const start = Math.max(from, reach);
    if (to > start) total += to - start;
    reach = Math.max(reach, to);
  }
  return total;
}

/**
 * One bar per UTC day of the last `days` (the last is today), from the notices' history: the worst
 * impact on the component that day and how long it was under a partial or major outage (overlapping
 * notices count once; degraded performance isn't downtime).
 */
export function uptimeDays(componentId: string, notices: NoticeHistory[], now: number, days = UPTIME_DAYS, since?: number | null): UptimeDay[] {
  const today = Math.floor(now / DAY) * DAY;
  const all = notices.flatMap((n) => segments(componentId, n, now));
  return Array.from({ length: days }, (_, i) => {
    const start = today - (days - 1 - i) * DAY;
    const end = Math.min(start + DAY, now);
    const day = all.filter((s) => s.from < end && s.to > start);
    const down = day.filter((s) => s.impact === 'partial_outage' || s.impact === 'major_outage').map((s): [number, number] => [Math.max(s.from, start), Math.min(s.to, end)]);
    return { start, status: worstStatus(day.map((s) => s.impact)), downMs: unionLength(down), noData: since != null && end <= since };
  });
}

/** Share of the time covered by the bars (from `since`, up to now) without a partial or major outage, in percent. */
export function uptimePercent(days: UptimeDay[], now: number, since?: number | null): number {
  const total = days.reduce((sum, d) => sum + Math.max(0, Math.min(d.start + DAY, now) - Math.max(d.start, since ?? d.start)), 0);
  const down = days.reduce((sum, d) => sum + d.downMs, 0);
  return total > 0 ? 100 * (1 - down / total) : 100;
}
