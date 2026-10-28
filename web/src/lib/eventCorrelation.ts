// Triage Inbox correlation with Event Management settings, for the mock (the API's is
// routes/event_correlation.rs, with the same rules): the first matching correlator groups an event by
// its group-by values while events keep arriving within its window; other events group per affected
// object; a work item whose events all fall in a maintenance window is suppressed.

export interface CorrelationEvent {
  timestamp: number;
  title: string;
  text: string;
  alertType: string;
  priority: string;
  source: string;
  host: string;
  aggregationKey: string;
  tags: string[];
}

export interface CorrelatorRule {
  id: string;
  name: string;
  query: string;
  groupBy: string[];
  windowMinutes: number;
}

export interface WindowRule {
  name: string;
  query: string;
  startsAt: number;
  endsAt: number;
}

export interface CorrelatedGroup<E> {
  key: string;
  correlator: CorrelatorRule | null;
  values: string[];
  events: E[];
  maintenance: string | null;
}

/** Whether the event matches the search: free text, source:, host:, status:, priority:, key:value tags, `-` negates. */
export function eventMatches(query: string, e: CorrelationEvent): boolean {
  const haystack = `${e.title} ${e.text}`.toLowerCase();
  const text: string[] = [];
  for (const raw of query.split(/\s+/).filter(Boolean)) {
    const neg = raw.startsWith('-');
    const tok = neg ? raw.slice(1) : raw;
    const i = tok.indexOf(':');
    const [k, v] = i > 0 ? [tok.slice(0, i), tok.slice(i + 1)] : ['', ''];
    let hit: boolean;
    if (k === 'source') hit = e.source === v;
    else if (k === 'host') hit = e.host === v;
    else if (k === 'status' || k === 'alert_type') hit = e.alertType === v;
    else if (k === 'priority') hit = e.priority === v;
    else if (k && v) hit = e.tags.includes(tok);
    else if (neg) hit = haystack.includes(tok.toLowerCase());
    else {
      text.push(tok);
      continue;
    }
    if (hit === neg) return false;
  }
  return text.length === 0 || haystack.includes(text.join(' ').toLowerCase());
}

/** An event's value for a group-by key: source, host, status, title or a tag's value. */
export function groupValue(e: CorrelationEvent, key: string): string {
  if (key === 'source') return e.source;
  if (key === 'host') return e.host;
  if (key === 'status') return e.alertType;
  if (key === 'title') return e.title;
  return e.tags.find((t) => t.startsWith(`${key}:`))?.slice(key.length + 1) ?? '';
}

export function correlateEvents<E extends CorrelationEvent>(events: E[], correlators: CorrelatorRule[], windows: WindowRule[]): CorrelatedGroup<E>[] {
  const out: CorrelatedGroup<E>[] = [];
  const open = new Map<string, number>();
  const marks: (string | null)[] = [];
  for (const e of [...events].sort((a, b) => a.timestamp - b.timestamp)) {
    const window = windows.find((w) => e.timestamp >= w.startsAt && e.timestamp <= w.endsAt && eventMatches(w.query, e))?.name ?? null;
    const c = correlators.find((x) => eventMatches(x.query, e)) ?? null;
    const values = c ? c.groupBy.map((k) => groupValue(e, k)) : [];
    const base = c ? `correlator:${c.id}:${values.join('\u001f')}` : e.aggregationKey || `${e.source}:${e.title}`;
    let wi = open.get(base);
    if (wi !== undefined && c) {
      const last = out[wi].events[out[wi].events.length - 1];
      if (e.timestamp - last.timestamp > c.windowMinutes * 60_000) wi = undefined;
    }
    if (wi === undefined) {
      out.push({ key: c && open.has(base) ? `${base}:${e.timestamp}` : base, correlator: c, values, events: [], maintenance: null });
      marks.push(window);
      wi = out.length - 1;
      open.set(base, wi);
    }
    out[wi].events.push(e);
    marks[wi] = window === null ? null : marks[wi] === null ? null : window;
  }
  out.forEach((g, i) => (g.maintenance = marks[i]));
  return out;
}

/** A correlated group's title: the correlator and its values, or the newest event's title. */
export function groupTitle<E extends CorrelationEvent>(g: CorrelatedGroup<E>): string {
  const values = g.values.filter(Boolean);
  if (g.correlator) return values.length ? `${g.correlator.name}: ${values.join(', ')}` : g.correlator.name;
  return g.events[g.events.length - 1]?.title ?? '';
}

export type InboxView = 'all' | 'open' | 'closed' | 'suppressed';

/** Whether a work item belongs in an inbox view: suppressed ones (in a maintenance window) only in All and Suppressed. */
export const inView = (i: { state: string; maintenance?: string | null }, view: InboxView): boolean =>
  view === 'all' ? true : view === 'suppressed' ? Boolean(i.maintenance) : !i.maintenance && i.state === view;
