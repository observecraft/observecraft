// Watchdog helpers shared by the page and the mock: filtering alerts with the search syntax,
// facet counts, labels, value formatting, the create-monitor link and the card sparkline.
// The alerts come from routes/watchdog.rs (or mock/watchdog.ts).
import type { components } from '../api/generated/schema';
import type { Facet, FacetValue } from '../api/types';
import { parseQuery, type Term } from './searchQuery';

export type WatchdogAlert = components['schemas']['WatchdogAlert'];
export type WatchdogPoint = components['schemas']['WatchdogPoint'];
export type WatchdogAlerts = components['schemas']['WatchdogAlerts'];

/** Facet values in display order. Third-party alerts have no source yet; the value is listed so the facet reads as complete. */
export const CATEGORIES: Record<string, string> = { apm: 'APM', infrastructure: 'Infrastructure', logs: 'Logs', third_party: 'Third Party' };
export const ALERT_TYPES: Record<string, string> = { error_rate: 'Error rate', latency: 'Latency', error_logs: 'Error logs', cpu: 'CPU usage', memory: 'Memory usage', disk: 'Disk usage' };
export const STATUSES: Record<string, string> = { ongoing: 'Ongoing', expired: 'Expired', resolved: 'Resolved' };

const FACETS: { key: string; label: string; values: Record<string, string> }[] = [
  { key: 'category', label: 'Alert Category', values: CATEGORIES },
  { key: 'type', label: 'Alert Type', values: ALERT_TYPES },
  { key: 'status', label: 'Alert Status', values: STATUSES },
];

/** The display label of a facet value. */
export function valueLabel(key: string, value: string): string {
  return FACETS.find((f) => f.key === key)?.values[value] ?? value;
}

/** An alert's value for a search key: its fields first, then its scope tags. */
function fieldValues(a: WatchdogAlert, key: string): string[] {
  switch (key) {
    case 'category':
      return [a.category];
    case 'type':
      return [a.alertType];
    case 'status':
      return [a.status];
    case 'team':
      return a.team ? [a.team] : [];
    case 'service':
      return a.service ? [a.service] : [];
    case 'host':
      return a.host ? [a.host] : [];
    default:
      return a.scope.filter((s) => s.startsWith(`${key}:`)).map((s) => s.slice(key.length + 1));
  }
}

/** `value` against a pattern with `*` wildcards, case-insensitive. */
function globMatch(pattern: string, value: string): boolean {
  if (!pattern.includes('*')) return pattern.toLowerCase() === value.toLowerCase();
  const re = new RegExp(
    `^${pattern
      .split('*')
      .map((p) => p.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
      .join('.*')}$`,
    'i',
  );
  return re.test(value);
}

function termMatches(a: WatchdogAlert, t: Term): boolean {
  const hit = t.kind === 'text' ? `${a.title} ${a.scope.join(' ')}`.toLowerCase().includes(t.text.toLowerCase()) : fieldValues(a, t.key).some((v) => t.values.some((p) => globMatch(p, v)));
  return t.negate ? !hit : hit;
}

function matches(a: WatchdogAlert, terms: Term[], skipKey?: string): boolean {
  return terms.every((t) => (t.kind === 'tag' && t.key === skipKey) || termMatches(a, t));
}

/** Alerts matching a search (`category:apm status:ongoing order-service`), in their order. */
export function filterAlerts(alerts: WatchdogAlert[], query: string): WatchdogAlert[] {
  const terms = parseQuery(query);
  return alerts.filter((a) => matches(a, terms));
}

/** Category, type and status facets with every known value; each counts the alerts matching the rest of the search. */
export function alertFacets(alerts: WatchdogAlert[], query: string): Facet[] {
  const terms = parseQuery(query);
  return FACETS.map((f) => {
    const shown = alerts.filter((a) => matches(a, terms, f.key));
    const values: FacetValue[] = Object.keys(f.values).map((value) => ({ value, count: shown.filter((a) => fieldValues(a, f.key).includes(value)).length }));
    return { key: f.key, label: f.label, group: 'ALL ALERTS', values };
  });
}

/** Teams of the alerts matching the rest of the search, the most alerted first. */
export function teamCounts(alerts: WatchdogAlert[], query: string): FacetValue[] {
  const terms = parseQuery(query);
  const counts = new Map<string, number>();
  for (const a of alerts) if (a.team && matches(a, terms, 'team')) counts.set(a.team, (counts.get(a.team) ?? 0) + 1);
  return [...counts.entries()].map(([value, count]) => ({ value, count })).sort((x, y) => y.count - x.count || x.value.localeCompare(y.value));
}

/** The create-monitor form prefilled from the anomaly: query, name and the top of the expected range as threshold. */
export function monitorLink(a: WatchdogAlert): string {
  const m = a.monitor;
  const isLog = m.monitorType === 'log';
  const p = new URLSearchParams({
    query: m.query,
    name: m.name,
    critical: String(m.critical),
    // Error logs are counted per 5-minute bucket, the other series averaged over 10 minutes.
    window: isLog ? '5' : '10',
    aggregation: isLog ? 'sum' : 'avg',
    tags: a.scope.join(', '),
  });
  return `/monitors/create/${m.monitorType}?${p}`;
}

/** A value in the alert's unit. */
export function formatValue(v: number, unit: string): string {
  if (unit === 'percent') return `${v.toFixed(1)}%`;
  if (unit === 'ms') return v >= 1000 ? `${(v / 1000).toPrecision(3)} s` : `${Math.round(v)} ms`;
  return `${Math.round(v).toLocaleString('en-US')} ${unit}`;
}

/** `25 min`, `1 h 5 min`, `2 h`. */
export function formatDuration(ms: number): string {
  const min = Math.max(1, Math.round(ms / 60_000));
  const h = Math.floor(min / 60);
  const m = min % 60;
  if (!h) return `${m} min`;
  return m ? `${h} h ${m} min` : `${h} h`;
}

/** Other alerts on the same service or host. */
export function relatedAlerts(a: WatchdogAlert, alerts: WatchdogAlert[]): WatchdogAlert[] {
  return alerts.filter((o) => o.id !== a.id && ((a.service && o.service === a.service) || (a.host && o.host === a.host)));
}

const r1 = (n: number) => Math.round(n * 10) / 10;

/** SVG path of the points in a `width`×`height` box (0 to the highest value, 1 px padding; gaps break the line) and the anomalous window's x and width. */
export function sparkline(points: WatchdogPoint[], start: number, end: number, width: number, height: number): { path: string; windowX: number; windowWidth: number } {
  if (points.length < 2) return { path: '', windowX: 0, windowWidth: 0 };
  const t0 = points[0].t;
  const t1 = points[points.length - 1].t;
  const max = Math.max(...points.map((p) => p.value ?? 0), 1e-9);
  const x = (t: number) => r1(((Math.min(Math.max(t, t0), t1) - t0) / (t1 - t0)) * width);
  const y = (v: number) => r1(1 + (1 - v / max) * (height - 2));
  let path = '';
  let pen = false;
  for (const p of points) {
    if (p.value === null || p.value === undefined) {
      pen = false;
      continue;
    }
    path += `${pen ? 'L' : 'M'}${x(p.t)},${y(p.value)}`;
    pen = true;
  }
  const windowX = x(start);
  return { path, windowX, windowWidth: r1(x(end) - windowX) };
}
