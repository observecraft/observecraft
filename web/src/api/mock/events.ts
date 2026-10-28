// In-browser events for mock mode: deployments, Kubernetes events, monitor alerts and agent events of
// the e-commerce demo, with the same query semantics as the API (free text, key:value, -key:value).
import type { EventItem, EventPage, EventSearchRequest } from '../events/client';
import { correlateEvents, groupTitle } from '../../lib/eventCorrelation';
import { activeCorrelators, valuesOf, windowsBetween } from './eventSettings';

const SOURCES = [
  {
    source: 'Kubernetes',
    alert: 'warning',
    title: (i: number) => `Back-off restarting failed container in pod checkout-7d9f${i % 7}`,
    text: 'Readiness probe failed: HTTP probe failed with statuscode: 503',
    tags: ['kube_namespace:shop', 'kube_deployment:checkout', 'env:prod'],
  },
  {
    source: 'Kubernetes',
    alert: 'info',
    title: (i: number) => `Scaled up replica set storefront-api-5c${i % 9} to 4`,
    text: 'Deployment storefront-api scaled from 3 to 4 replicas',
    tags: ['kube_namespace:shop', 'kube_deployment:storefront-api', 'env:prod'],
  },
  {
    source: 'Change Tracking',
    alert: 'info',
    title: (i: number) => `order-service deployed version 1.4.${i % 5}`,
    text: 'Rolled out by the delivery pipeline',
    tags: ['service:order-service', 'env:prod', 'version:1.4.2'],
  },
  {
    source: 'Monitor Alert',
    alert: 'error',
    title: () => '[Triggered] High error rate on billing',
    text: 'trace.http.request.errors for service:billing is above 5%',
    tags: ['service:billing', 'env:prod', 'monitor'],
  },
  {
    source: 'Monitor Alert',
    alert: 'success',
    title: () => '[Recovered] High error rate on billing',
    text: 'trace.http.request.errors for service:billing is back below 5%',
    tags: ['service:billing', 'env:prod', 'monitor'],
  },
  {
    source: 'containerd',
    alert: 'info',
    title: (i: number) => `Image pulled: example-org/catalog-service:2.${i % 4}.0`,
    text: 'Pulled image in 1.2s',
    tags: ['image_name:example-org/catalog-service', 'env:prod'],
  },
  { source: 'System', alert: 'info', title: () => 'Agent Startup', text: 'Version 7.84.6', tags: [] },
];

function demo(now: number): EventItem[] {
  const out: EventItem[] = [];
  for (let i = 0; i < 320; i++) {
    const s = SOURCES[(i * 3) % SOURCES.length];
    const ts = now - i * 47_000 - (i % 5) * 3_100;
    out.push({
      id: `ev-${i}`,
      timestamp: ts,
      title: s.title(i),
      text: s.text,
      alertType: s.alert,
      priority: s.alert === 'error' ? 'normal' : i % 4 === 0 ? 'low' : 'normal',
      source: s.source,
      host: ['web-1', 'web-2', 'db-1', 'worker-1'][i % 4],
      aggregationKey: '',
      tags: s.tags,
    });
  }
  return out;
}

function matches(e: EventItem, token: string): boolean {
  const neg = token.startsWith('-');
  const t = neg ? token.slice(1) : token;
  const i = t.indexOf(':');
  let hit: boolean;
  if (i > 0) {
    const k = t.slice(0, i);
    const v = t.slice(i + 1);
    if (k === 'source') hit = e.source.toLowerCase() === v.toLowerCase();
    else if (k === 'host') hit = e.host === v;
    else if (k === 'status') hit = e.alertType === v;
    else if (k === 'priority') hit = e.priority === v;
    else hit = e.tags.includes(`${k}:${v}`);
  } else {
    hit = `${e.title} ${e.text}`.toLowerCase().includes(t.toLowerCase());
  }
  return neg ? !hit : hit;
}

export async function searchEvents(req: EventSearchRequest): Promise<EventPage> {
  const tokens = (req.query ?? '').split(/\s+/).filter(Boolean);
  const all = demo(req.range.to).filter((e) => e.timestamp >= req.range.from && e.timestamp <= req.range.to && tokens.every((t) => matches(e, t)));
  const facet = (key: string, label: string, get: (e: EventItem) => string | undefined, group = 'CORE') => {
    const counts = new Map<string, number>();
    for (const e of all) {
      const v = get(e);
      if (v) counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    return { key, label, group, values: [...counts].sort((a, b) => b[1] - a[1]).map(([value, count]) => ({ value, count })) };
  };
  // Tag facets: env and service, then the ones added with "+ Add".
  const tagKeys = [...new Set(['env', 'service', ...(req.extra ?? [])])].filter((k) => !['source', 'status', 'priority', 'host'].includes(k));
  const tagValue = (e: EventItem, key: string) => e.tags.find((t) => t.startsWith(`${key}:`))?.slice(key.length + 1);
  const span = Math.max(60_000, req.range.to - req.range.from);
  const step = [10_000, 30_000, 60_000, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000, 43_200_000, 86_400_000].find((s) => span / s <= 90) ?? 86_400_000;
  const histogram = [];
  for (let t = Math.floor(req.range.from / step) * step; t <= req.range.to; t += step) {
    const counts: Record<string, number> = {};
    for (const e of all) if (e.timestamp >= t && e.timestamp < t + step) counts[e.alertType] = (counts[e.alertType] ?? 0) + 1;
    histogram.push({ t, counts });
  }
  return {
    items: all.slice(req.offset ?? 0, (req.offset ?? 0) + req.limit),
    total: all.length,
    facets: [
      facet('source', 'Source', (e) => e.source),
      facet('status', 'Status', (e) => e.alertType),
      facet('priority', 'Priority', (e) => e.priority),
      facet('host', 'Host', (e) => e.host),
      ...tagKeys.map((k) => facet(k, k === 'env' ? 'Env' : k === 'service' ? 'Service' : k, (e) => tagValue(e, k), 'TAGS')),
    ],
    histogram,
  };
}

/** Tag keys of the matching events, most common first. */
export async function eventFields(req: { range: EventSearchRequest['range']; query?: string }): Promise<{ key: string; count: number }[]> {
  const { items } = await searchEvents({ range: req.range, query: req.query ?? '', limit: 10_000, offset: 0 });
  const counts = new Map<string, number>();
  for (const e of items) for (const key of new Set(e.tags.filter((t) => t.includes(':')).map((t) => t.slice(0, t.indexOf(':'))))) counts.set(key, (counts.get(key) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([key, count]) => ({ key, count }));
}

/**
 * Warning and error events correlated into work items: by the correlators of Event Management →
 * Settings (mock/eventSettings.ts) when they match, else per source and title (the demo events have
 * no aggregation keys); maintenance windows suppress the work items they cover.
 */
export async function workItems(req: { range: EventSearchRequest['range']; query?: string }) {
  const { items } = await searchEvents({ range: req.range, query: req.query ?? '', limit: 10_000, offset: 0 });
  const correlators = activeCorrelators().map((c) => ({ id: c.id, name: c.name, query: c.query, groupBy: c.groupBy, windowMinutes: c.windowMinutes }));
  const windows = windowsBetween(req.range.from, req.range.to);
  const groups = correlateEvents(
    items.filter((e) => e.alertType === 'error' || e.alertType === 'warning'),
    correlators,
    windows,
  );
  const list = groups.map((g) => {
    const es = g.events;
    const newest = es[es.length - 1];
    const last = newest.timestamp;
    return {
      id: g.key,
      title: groupTitle(g),
      source: newest.source,
      status: es.some((e) => e.alertType === 'error') ? 'error' : 'warning',
      reason: '',
      summary: newest.text.split('\n')[0],
      count: es.length,
      firstSeen: es[0].timestamp,
      lastSeen: last,
      state: req.range.to - last <= 30 * 60_000 ? 'open' : 'closed',
      tags: newest.tags.filter((t) => /^(kube_cluster_name|kube_namespace|service|env):/.test(t)),
      correlator: g.correlator?.name ?? null,
      maintenance: g.maintenance,
      attributes: valuesOf(g.key),
    };
  });
  list.sort((a, b) => b.lastSeen - a.lastSeen);
  const suppressed = list.filter((i) => i.maintenance).length;
  const open = list.filter((i) => !i.maintenance && i.state === 'open').length;
  return { items: list, open, closed: list.length - suppressed - open, suppressed };
}
