// Service check helpers shared by Check Summary and the mock (the server's twin is routes/service_checks.rs).

/** Words written in capitals in check names. */
const ACRONYMS = new Set(['api', 'aws', 'cpu', 'dns', 'gcp', 'http', 'https', 'io', 'ntp', 'os', 'sql', 'ssh', 'ssl', 'tcp', 'tls', 'url']);

/** A readable name for a check: `datadog.agent.up` → `Datadog Agent Up`, `ntp.in_sync` → `NTP In Sync`. */
export function checkDisplayName(check: string): string {
  return check
    .split(/[._-]/)
    .filter(Boolean)
    .map((w) => (ACRONYMS.has(w.toLowerCase()) ? w.toUpperCase() : w[0].toUpperCase() + w.slice(1)))
    .join(' ');
}

export const STATUS_LABELS = ['OK', 'WARNING', 'CRITICAL', 'UNKNOWN'] as const;

/** 0 OK, 1 WARNING, 2 CRITICAL, 3 UNKNOWN → its label. */
export const statusLabel = (status: number): string => STATUS_LABELS[status] ?? 'UNKNOWN';

/** CSS modifier for a status. */
export const statusClass = (status: number | null | undefined): string => (status === null || status === undefined ? 'none' : (['ok', 'warning', 'critical', 'unknown'][status] ?? 'unknown'));

/** Checks whose name or display name contains the search, sorted by `key` in `dir`. */
export function filterChecks<T extends { check: string; name: string }>(checks: T[], search: string, key: 'check' | 'name', dir: 'asc' | 'desc'): T[] {
  const s = search.trim().toLowerCase();
  const shown = checks.filter((c) => !s || c.check.toLowerCase().includes(s) || c.name.toLowerCase().includes(s));
  return shown.sort((a, b) => (dir === 'asc' ? 1 : -1) * a[key].localeCompare(b[key]));
}

/**
 * A metric monitor on the check: alerts when any host reports CRITICAL (warns on WARNING), as the
 * `service_check.status` gauge.
 */
export function checkMonitorLink(check: string): string {
  const p = new URLSearchParams({
    query: `max:service_check.status{check:${check}} by {host}`,
    name: `${checkDisplayName(check)} is failing on {{host.name}}`,
    critical: '2',
    warning: '1',
    aggregation: 'max',
    window: '5',
  });
  return `/monitors/create/metric?${p}`;
}
