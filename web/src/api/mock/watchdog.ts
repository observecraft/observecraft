// In-browser Watchdog alerts for mock mode: anomalies on the e-commerce demo services and hosts,
// deterministic and relative to now, shaped like routes/watchdog.rs (5-minute buckets, the series
// two hours either side of the anomaly with its expected range).
import type { WatchdogAlert, WatchdogAlerts } from '../../lib/watchdog';

const BUCKET = 5 * 60_000;
const CONTEXT = 24;
const HOUR = 3_600_000;

type Spec = {
  alertType: WatchdogAlert['alertType'];
  key: string;
  team?: string;
  /** Start, hours ago, and length in buckets. */
  agoHours: number;
  buckets: number;
  status: WatchdogAlert['status'];
  baseline: number;
  peak: number;
  datadog?: boolean;
};

const SPECS: Spec[] = [
  { alertType: 'error_rate', key: 'order-service', team: 'checkout', agoHours: 0.5, buckets: 6, status: 'ongoing', baseline: 0.6, peak: 14.2 },
  { alertType: 'cpu', key: 'i-0a1b2c3d4e5f60003', agoHours: 0.75, buckets: 9, status: 'ongoing', baseline: 31, peak: 94, datadog: true },
  { alertType: 'latency', key: 'order-service', team: 'checkout', agoHours: 0.4, buckets: 5, status: 'ongoing', baseline: 182, peak: 940 },
  { alertType: 'latency', key: 'storefront-api', team: 'storefront', agoHours: 3, buckets: 9, status: 'resolved', baseline: 96, peak: 610 },
  { alertType: 'error_logs', key: 'billing', team: 'payments', agoHours: 6, buckets: 7, status: 'resolved', baseline: 4, peak: 260 },
  { alertType: 'memory', key: 'i-0a1b2c3d4e5f60005', agoHours: 10, buckets: 14, status: 'expired', baseline: 58, peak: 91 },
  { alertType: 'disk', key: 'i-0a1b2c3d4e5f60002', agoHours: 14, buckets: 20, status: 'resolved', baseline: 71, peak: 83, datadog: true },
  { alertType: 'error_rate', key: 'inventory-service', team: 'catalog', agoHours: 20, buckets: 4, status: 'resolved', baseline: 0.3, peak: 9.8 },
];

const TYPE = {
  error_rate: { category: 'apm', unit: 'percent', title: 'Error rate increase', name: 'Error rate', delta: 5, ratio: 2 },
  latency: { category: 'apm', unit: 'ms', title: 'Latency increase', name: 'p95 latency', delta: 50, ratio: 1.5 },
  error_logs: { category: 'logs', unit: 'logs', title: 'Error logs increase', name: 'Error log volume', delta: 20, ratio: 3 },
  cpu: { category: 'infrastructure', unit: 'percent', title: 'CPU usage increase', name: 'CPU usage', delta: 20, ratio: 1 },
  memory: { category: 'infrastructure', unit: 'percent', title: 'Memory usage increase', name: 'Memory usage', delta: 10, ratio: 1 },
  disk: { category: 'infrastructure', unit: 'percent', title: 'Disk usage increase', name: 'Disk usage', delta: 5, ratio: 1 },
} as const;

/** The same monitor queries the server suggests. */
function monitorQuery(s: Spec): { monitorType: string; query: string } {
  const k = s.key;
  switch (s.alertType) {
    case 'error_rate':
      return { monitorType: 'apm', query: `avg:trace.error_rate{service:${k}}` };
    case 'latency':
      return { monitorType: 'apm', query: `avg:trace.latency.p95{service:${k}}` };
    case 'error_logs':
      return { monitorType: 'log', query: `logs("service:${k} status:error") by {service}` };
    case 'cpu':
      return { monitorType: 'metric', query: s.datadog ? `100 - avg:system.cpu.idle{host:${k}}` : `100 - avg:system.cpu.utilization{host:${k},state:idle} * 100` };
    case 'memory':
      return { monitorType: 'metric', query: s.datadog ? `100 - avg:system.mem.pct_usable{host:${k}} * 100` : `avg:system.memory.utilization{host:${k},state:used} * 100` };
    case 'disk':
      return { monitorType: 'metric', query: s.datadog ? `max:system.disk.in_use{host:${k}} * 100` : `max:system.filesystem.utilization{host:${k}} * 100` };
  }
}

const round = (v: number) => Math.round(v * 100) / 100;

function build(s: Spec, now: number): WatchdogAlert {
  const t = TYPE[s.alertType];
  const last = Math.floor(now / BUCKET) * BUCKET - BUCKET;
  const start = Math.floor((now - s.agoHours * HOUR) / BUCKET) * BUCKET;
  const endBucket = Math.min(start + (s.buckets - 1) * BUCKET, last);
  const noise = (i: number) => [0, 0.06, -0.05, 0.03, -0.02, 0.04, -0.06][i % 7] * Math.max(s.baseline, 1);
  const spread = Math.max(s.baseline, 1) * 0.08 * 4;
  const upper = round(s.baseline + Math.max(spread, t.delta, s.baseline * (t.ratio - 1)));
  const lower = round(Math.max(0, s.baseline - spread));
  // Expired series stop reporting after the anomaly.
  const to = s.status === 'ongoing' ? last : Math.min(endBucket + CONTEXT * BUCKET, last);
  const points = [];
  for (let ts = start - CONTEXT * BUCKET, i = 0; ts <= to; ts += BUCKET, i++) {
    const inWindow = ts >= start && ts <= endBucket;
    // The anomaly ramps up to its peak and holds.
    const k = inWindow ? Math.min(1, (ts - start) / BUCKET / 2 + 0.5) : 0;
    let value: number | null = inWindow ? s.baseline + (s.peak - s.baseline) * k + noise(i) : s.baseline + noise(i);
    if (s.status === 'expired' && ts > endBucket) value = null;
    points.push({ t: ts, value: value === null ? null : round(value), lower: value === null ? null : lower, upper: value === null ? null : upper });
  }
  const scope = `${t.category === 'infrastructure' ? 'host' : 'service'}:${s.key}`;
  const onHost = t.category === 'infrastructure';
  return {
    id: `${s.alertType}:${scope}:${start}`,
    category: t.category,
    alertType: s.alertType,
    status: s.status,
    title: `${t.title} on ${scope}`,
    scope: [scope],
    service: onHost ? null : s.key,
    host: onHost ? s.key : null,
    team: s.team ?? null,
    unit: t.unit,
    start,
    end: endBucket + BUCKET,
    baseline: s.baseline,
    peak: s.peak,
    points,
    monitor: { ...monitorQuery(s), name: `${t.name} is high on ${s.key}`, critical: s.alertType === 'error_rate' ? upper : Math.ceil(upper) },
  };
}

export async function watchdogAlerts(now = Date.now()): Promise<WatchdogAlerts> {
  const alerts = SPECS.map((s) => build(s, now));
  const ongoing = (a: WatchdogAlert) => (a.status === 'ongoing' ? 1 : 0);
  alerts.sort((a, b) => ongoing(b) - ongoing(a) || b.start - a.start);
  const to = Math.floor(now / BUCKET) * BUCKET;
  return { alerts, from: to - 24 * HOUR, to, bucketMs: BUCKET, services: 12, hosts: 6 };
}
