// Pure helpers of the service page (tested by scripts/apm-service-page.test.mjs): no React, no API calls.
import type { EnvRate, ServiceMonitor } from '../../../api/apm/client';

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
