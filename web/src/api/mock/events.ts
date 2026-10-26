// In-browser events for mock mode: deployments, Kubernetes events, monitor alerts and agent events of
// the e-commerce demo, with the same query semantics as the API (free text, key:value, -key:value).
import type { EventItem, EventPage, EventSearchRequest } from '../events/client';

const SOURCES = [
  { source: 'Kubernetes', alert: 'warning', title: (i: number) => `Back-off restarting failed container in pod checkout-7d9f${i % 7}`, text: 'Readiness probe failed: HTTP probe failed with statuscode: 503', tags: ['kube_namespace:shop', 'kube_deployment:checkout', 'env:prod'] },
  { source: 'Kubernetes', alert: 'info', title: (i: number) => `Scaled up replica set storefront-api-5c${i % 9} to 4`, text: 'Deployment storefront-api scaled from 3 to 4 replicas', tags: ['kube_namespace:shop', 'kube_deployment:storefront-api', 'env:prod'] },
  { source: 'Change Tracking', alert: 'info', title: (i: number) => `order-service deployed version 1.4.${i % 5}`, text: 'Rolled out by the delivery pipeline', tags: ['service:order-service', 'env:prod', 'version:1.4.2'] },
  { source: 'Monitor Alert', alert: 'error', title: () => '[Triggered] High error rate on billing', text: 'trace.http.request.errors for service:billing is above 5%', tags: ['service:billing', 'env:prod', 'monitor'] },
  { source: 'Monitor Alert', alert: 'success', title: () => '[Recovered] High error rate on billing', text: 'trace.http.request.errors for service:billing is back below 5%', tags: ['service:billing', 'env:prod', 'monitor'] },
  { source: 'containerd', alert: 'info', title: (i: number) => `Image pulled: example-org/catalog-service:2.${i % 4}.0`, text: 'Pulled image in 1.2s', tags: ['image_name:example-org/catalog-service', 'env:prod'] },
  { source: 'System', alert: 'info', title: () => 'Agent Startup', text: 'Version 7.84.6', tags: [] },
];

function demo(now: number): EventItem[] {
  const out: EventItem[] = [];
  for (let i = 0; i < 320; i++) {
    const s = SOURCES[(i * 7) % SOURCES.length];
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
    if (k === 'source') hit = e.source === v;
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
  const facet = (key: 'source' | 'status' | 'priority' | 'host', label: string, get: (e: EventItem) => string) => {
    const counts = new Map<string, number>();
    for (const e of all) counts.set(get(e), (counts.get(get(e)) ?? 0) + 1);
    return { key, label, group: 'CORE', values: [...counts].sort((a, b) => b[1] - a[1]).map(([value, count]) => ({ value, count })) };
  };
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
    facets: [facet('source', 'Source', (e) => e.source), facet('status', 'Status', (e) => e.alertType), facet('priority', 'Priority', (e) => e.priority), facet('host', 'Host', (e) => e.host)],
    histogram,
  };
}
