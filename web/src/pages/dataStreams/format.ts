// Number formats of Data Streams Monitoring.

const trim = (v: number, digits: number) => String(+v.toFixed(digits));

/** 4820 → "4.82K", 12 → "12". */
export function fmtCount(n: number | null | undefined): string {
  if (n == null) return '—';
  const a = Math.abs(n);
  if (a < 1000) return trim(n, a < 10 ? 1 : 0);
  if (a < 1e6) return `${trim(n / 1e3, 2)}K`;
  if (a < 1e9) return `${trim(n / 1e6, 2)}M`;
  return `${trim(n / 1e9, 2)}B`;
}

/** Messages per second. */
export function fmtRate(n: number | null | undefined): string {
  if (n == null) return '—';
  if (n > 0 && n < 0.01) return '< 0.01 msg/s';
  return `${n < 100 ? trim(n, 2) : fmtCount(n)} msg/s`;
}

/** Milliseconds: "0.42 ms", "9.4 ms", "1.45 s". */
export function fmtMs(ms: number | null | undefined): string {
  if (ms == null) return '—';
  if (ms >= 1000) return `${trim(ms / 1000, 2)} s`;
  return `${trim(ms, ms < 1 ? 2 : ms < 100 ? 1 : 0)} ms`;
}

/** Lag in time: "0.7s", "42s", "1m 36s", "2h 5m". */
export function fmtLagSeconds(s: number | null | undefined): string {
  if (s == null) return '—';
  if (s < 10) return `${trim(s, 1)}s`;
  if (s < 60) return `${Math.round(s)}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
  return `${Math.floor(s / 3600)}h ${Math.round((s % 3600) / 60)}m`;
}

/** Bytes per second. */
export function fmtBytesRate(n: number | null | undefined): string {
  if (n == null) return '—';
  const units = ['B/s', 'KiB/s', 'MiB/s', 'GiB/s'];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return `${trim(v, v < 10 ? 1 : 0)} ${units[u]}`;
}

/** The name in a node id (`service:<name>`, `topic:<system>:<name>`). */
export function nodeName(id: string): string {
  if (id.startsWith('topic:')) return id.split(':').slice(2).join(':');
  return id.replace(/^service:/, '');
}

export const isTopic = (id: string) => id.startsWith('topic:');

const SYSTEM_LOGOS: Record<string, string> = { kafka: 'apachekafka', rabbitmq: 'rabbitmq', sqs: 'amazonsqs', sns: 'amazonsns' };

/** The brand logo of a messaging system (`kafka`, `rabbitmq`, `sqs`, …), if there is one. */
export const systemLogo = (system: string | null | undefined): string | null => (system ? (SYSTEM_LOGOS[system.toLowerCase()] ?? null) : null);
