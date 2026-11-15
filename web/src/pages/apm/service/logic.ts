// Pure helpers of the service page (tested by scripts/apm-service-page.test.mjs): no React, no API calls.
import type { Dependency, EnvRate, ServiceMonitor } from '../../../api/apm/client';
import { quoteValue } from '../../../lib/searchQuery';
import { compact } from '../../../lib/format';

/** All envs (the selectors' `*`). */
export const ALL = '*';

/** An env's requests per second as the env selector shows it (`0.6`, `12.3`), `NO DATA` without any. */
export function envRateLabel(reqPerSec: number | null | undefined): string {
  if (reqPerSec == null) return 'NO DATA';
  if (reqPerSec === 0) return '0';
  if (reqPerSec < 0.01) return '<0.01';
  const digits = reqPerSec < 10 ? 100 : reqPerSec < 100 ? 10 : 1;
  return String(Math.round(reqPerSec * digits) / digits);
}

/** The env selector: `*` with every env's requests, then each env; the env in the URL stays listed. */
export function envOptions(envs: EnvRate[], current: string | null): { value: string; label: string; hint: string }[] {
  const listed = current && current !== ALL && !envs.some((e) => e.env === current) ? [...envs, { env: current, reqPerSec: null }] : envs;
  const total = envs.reduce((sum, e) => sum + (e.reqPerSec ?? 0), 0);
  return [{ value: ALL, label: ALL, hint: envRateLabel(total) }, ...listed.map((e) => ({ value: e.env, label: e.env, hint: envRateLabel(e.reqPerSec) }))];
}

/** A primary tag's values; the one in the URL stays listed. */
export function tagValues(values: readonly string[], current: string | null): string[] {
  return current && current !== ALL && !values.includes(current) ? [...values, current] : [...values];
}

type MonitorStatus = ServiceMonitor['status'];

/** Monitor statuses, the worst first, and how the monitor bar names them. */
export const MONITOR_STATUSES: { status: MonitorStatus; label: string }[] = [
  { status: 'alert', label: 'ALERT' },
  { status: 'warn', label: 'WARN' },
  { status: 'nodata', label: 'NO DATA' },
  { status: 'ok', label: 'OK' },
];

/** The monitor bar's segments (`ALERT: 2 Monitors`, `WARN: 1`, `OK: 4 Monitors`), the worst first; the bar sizes them by count. */
export function monitorSegments(monitors: Pick<ServiceMonitor, 'status'>[]): { status: MonitorStatus; count: number; label: string }[] {
  return MONITOR_STATUSES.map(({ status, label }) => {
    const count = monitors.filter((m) => m.status === status).length;
    const noun = status === 'alert' || status === 'ok' ? ` Monitor${count === 1 ? '' : 's'}` : '';
    return { status, count, label: `${label}: ${count}${noun}` };
  }).filter((s) => s.count > 0);
}

/** The status badges of a resource's monitors (`1 ALERT`, `4 OK`), the worst first; none without monitors. */
export function resourceMonitorBadges(monitors: Pick<ServiceMonitor, 'status' | 'resources'>[], resource: string): { status: MonitorStatus; count: number; label: string }[] {
  const mine = monitors.filter((m) => m.resources.includes(resource));
  return MONITOR_STATUSES.map(({ status, label }) => ({ status, count: mine.filter((m) => m.status === status).length, label })).flatMap((b) =>
    b.count ? [{ ...b, label: `${b.count} ${b.label}` }] : [],
  );
}

/** Deployment markers for a time chart: each version's first span in the range, but the range's first version (it
 * was running already) and those outside the series. */
export function deploymentMarkers(deployments: { version: string; firstSeen: number }[], series: { t: number }[]): { t: number; version: string }[] {
  if (series.length === 0) return [];
  const sorted = [...deployments].sort((a, b) => a.firstSeen - b.firstSeen);
  const [from, to] = [series[0].t, series[series.length - 1].t];
  return sorted
    .slice(1)
    .filter((d) => d.firstSeen >= from && d.firstSeen <= to)
    .map((d) => ({ t: d.firstSeen, version: d.version }));
}

/** A chart title's total: `1.1K total (0.3 req/s)`. */
export function totalLabel(count: number, perSec: number): string {
  return `${compact(count)} total (${envRateLabel(perSec)} req/s)`;
}

type ServiceKind = Dependency['kind'];

/** Databases, caches, queues and external hosts are inferred from callers' spans; the map draws them dashed. */
export const isInferred = (kind: ServiceKind): boolean => kind !== 'web' && kind !== 'worker';

/** A dependency's health from its error rate, as the server rates a service's. */
export function dependencyHealth(errorRate: number): 'ok' | 'warn' | 'critical' {
  return errorRate > 0.05 ? 'critical' : errorRate > 0.01 ? 'warn' : 'ok';
}

/** A dependency's share of its side's requests (callers' or calls'). */
export function trafficShare(d: { requests: number }, side: { requests: number }[]): number {
  const total = side.reduce((sum, x) => sum + x.requests, 0);
  return total ? d.requests / total : 0;
}

/** A map node's links: its service page, and the traces of the calls between it and `service`. */
export function dependencyLinks(service: string, d: Pick<Dependency, 'service' | 'kind'>, side: 'upstream' | 'downstream'): { service: string; traces: string } {
  const query = isInferred(d.kind) && side === 'downstream' ? `service:${quoteValue(service)} @peer.service:${quoteValue(d.service)}` : `service:${quoteValue(d.service)}`;
  return { service: `/apm/services/${encodeURIComponent(d.service)}`, traces: `/apm/traces?query=${encodeURIComponent(query)}` };
}

// ---------------------------------------------------------------- infrastructure

export type HostUnit = 'number' | 'percent' | 'fraction' | 'bytes' | 'kibibytes' | 'bytesPerSec';

export interface HostVariant {
  title: string;
  queries: (h: string) => string[];
  unit: HostUnit;
  stacked?: boolean;
}

/** Host charts in Datadog Agent metric names, with the OpenTelemetry hostmetrics equivalent tried when the Agent's are absent. */
export const HOST_CHARTS: { id: string; variants: HostVariant[] }[] = [
  {
    id: 'queue',
    variants: [
      // linux_proc_extras, which the Agent doesn't run by default; else its load check, else hostmetrics.
      { title: 'Process Queue Length', queries: (h) => [`avg:system.proc.queue_length{${h}}`], unit: 'number' },
      { title: 'Load average (1m)', queries: (h) => [`avg:system.load.1{${h}}`], unit: 'number' },
      { title: 'Load average (1m)', queries: (h) => [`avg:system.cpu.load_average.1m{${h}}`], unit: 'number' },
    ],
  },
  {
    id: 'disk-queue',
    variants: [
      { title: 'Disk Queue Length (by device)', queries: (h) => [`avg:system.io.avg_q_sz{${h}} by {device}`], unit: 'number' },
      { title: 'Disk Queue Length (by device)', queries: (h) => [`avg:system.disk.pending_operations{${h}} by {device}`], unit: 'number' },
    ],
  },
  {
    id: 'cpu',
    variants: [
      { title: 'CPU usage (%)', queries: (h) => [`avg:system.cpu.user{${h}}`, `avg:system.cpu.system{${h}}`, `avg:system.cpu.iowait{${h}}`], unit: 'percent', stacked: true },
      { title: 'CPU usage (%)', queries: (h) => [`avg:system.cpu.utilization{${h},!state:idle} by {state}`], unit: 'fraction', stacked: true },
    ],
  },
  {
    id: 'memory',
    variants: [
      // The Agent reports memory in MiB.
      { title: 'Memory breakdown (MiB)', queries: (h) => [`sum:system.mem.used{${h}}`, `sum:system.mem.total{${h}}`], unit: 'number' },
      { title: 'Memory breakdown', queries: (h) => [`sum:system.memory.usage{${h}} by {state}`], unit: 'bytes', stacked: true },
    ],
  },
  {
    id: 'network',
    variants: [
      { title: 'Network traffic (bytes per sec)', queries: (h) => [`sum:system.net.bytes_rcvd{${h}}`, `sum:system.net.bytes_sent{${h}}`], unit: 'bytesPerSec' },
      { title: 'Network traffic (bytes per sec)', queries: (h) => [`sum:system.network.io{${h}} by {direction}`], unit: 'bytesPerSec' },
    ],
  },
  {
    id: 'disk',
    variants: [
      // The Agent's disk check reports KiB.
      { title: 'Disk used (by device)', queries: (h) => [`max:system.disk.used{${h}} by {device}`], unit: 'kibibytes' },
      // OpenTelemetry hostmetrics: bytes in use (on by default), then the utilization (opt-in).
      { title: 'Disk used (by device)', queries: (h) => [`max:system.filesystem.usage{${h},state:used} by {device}`], unit: 'bytes' },
      { title: 'Disk used (%) (by mountpoint)', queries: (h) => [`max:system.filesystem.utilization{${h}} by {mountpoint}`], unit: 'fraction' },
    ],
  },
];
