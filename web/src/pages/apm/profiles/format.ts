// Value formats and frame helpers of the profiling pages.
import type { ProfileMetric } from '../../../api/profiles/client';
import { compact } from '../../../lib/format';
import { formatBytes } from '../../../components/ui/Pager';

/** Nanoseconds as Datadog writes durations: "2m 29s", "1.11s", "525ms", "34µs". */
export function fmtNanos(ns: number): string {
  const ms = ns / 1e6;
  if (ms >= 60_000) {
    const m = Math.floor(ms / 60_000);
    return `${m}m ${Math.round((ms - m * 60_000) / 1000)}s`;
  }
  if (ms >= 1000) return `${(ms / 1000).toFixed(ms >= 10_000 ? 1 : 2)}s`;
  if (ms >= 1) return `${Math.round(ms)}ms`;
  if (ns >= 1000) return `${Math.round(ns / 1000)}µs`;
  return `${Math.round(ns)}ns`;
}

/** A sample-type value in its unit. */
export function fmtValue(v: number, unit: string): string {
  if (unit === 'nanoseconds') return fmtNanos(v);
  if (unit === 'bytes') return formatBytes(v);
  return compact(Math.round(v));
}

/** "1.11s per minute" for summed types; "127 MiB" for gauges. */
export function fmtCard(v: number, m: Pick<ProfileMetric, 'unit' | 'gauge'>): string {
  return m.gauge ? fmtValue(v, m.unit) : `${fmtValue(v, m.unit)} per minute`;
}

/** "Average CPU Time, per minute" / "Average Heap Live Size". */
export function averageLabel(m: Pick<ProfileMetric, 'label' | 'gauge'>): string {
  return m.gauge ? `Average ${m.label}` : `Average ${m.label}, per minute`;
}

/** The chart's name of a metric: CPU time is shown in cores (Datadog's "CPU - Cores"). */
export function chartLabel(m: ProfileMetric): string {
  if (m.type === 'cpu-time') return 'CPU - Cores';
  if (m.type === 'wall-time') return 'Wall Time - Threads';
  return m.gauge ? m.label : `${m.label} - per minute`;
}

/**
 * Chart values from the API's per-second values: cores/threads for nanosecond types, per minute
 * for the other summed types, as is for gauges.
 */
export function chartValue(perSecond: number, m: ProfileMetric): number {
  if (m.gauge) return perSecond;
  if (m.unit === 'nanoseconds') return perSecond / 1e9;
  return perSecond * 60;
}

export function fmtChart(v: number, m: ProfileMetric): string {
  if (!m.gauge && m.unit === 'nanoseconds') return v < 1 ? `${(v * 1000).toFixed(v < 0.01 ? 2 : 1)} mcores` : `${v.toFixed(2)} cores`;
  return fmtValue(v, m.unit);
}

/** Display name of a frame: "function (file.py)" for scripting languages, whose names lack the module. */
export function frameLabel(name: string, file: string): string {
  if (!/\.(py|rb|js|php)$/.test(file)) return name;
  return `${name} (${file.split('/').pop()})`;
}

/**
 * The library a frame belongs to: Python distribution (or `stdlib`), Java/.NET namespace, Go import
 * path; empty for application code. Mirrors the server's `profiles::package`.
 */
export function framePackage(name: string, file: string): string {
  const m = /(?:site|dist)-packages\/([^/]+)/.exec(file);
  if (m) return m[1].replace(/\.py$/, '');
  if (file.includes('/lib/python') || file.startsWith('<frozen')) return 'stdlib';
  if (/\.(py|rb|js|php)$/.test(file)) return '';
  if (name.includes('/')) {
    const i = name.lastIndexOf('/');
    return `${name.slice(0, i)}/${name.slice(i + 1).split('.')[0]}`;
  }
  const parts = name.split('(')[0].split('.');
  if (parts.length > 2) return parts.slice(0, -2).join('.');
  const i = name.indexOf('::');
  return i > 0 ? name.slice(0, i) : '';
}

/** Runtime and framework namespaces (JVM, .NET) folded like Python's stdlib and site-packages. */
const LIBRARY_PREFIXES = [
  'java.',
  'javax.',
  'jdk.',
  'sun.',
  'com.sun.',
  'kotlin.',
  'kotlinx.',
  'scala.',
  'org.springframework',
  'org.eclipse',
  'org.apache',
  'org.hibernate',
  'org.slf4j',
  'com.fasterxml',
  'com.google',
  'com.zaxxer',
  'io.netty',
  'io.micrometer',
  'io.grpc',
  'reactor.',
  'ch.qos',
  'okhttp3',
  'System',
  'Microsoft',
  'Npgsql',
  'Newtonsoft',
  'Grpc.',
  'golang.org/',
  'google.golang.org/',
];

/**
 * Whether a frame's package is a library (folded unless Expand All) rather than application code:
 * Python stdlib and site-packages, the Go standard library (import paths without a domain), and
 * well-known runtime and framework namespaces.
 */
export function isLibrary(pkg: string, file = ''): boolean {
  if (!pkg) return false;
  if (/(?:site|dist)-packages\//.test(file) || pkg === 'stdlib') return true;
  if (/\.go$|\.s$/.test(file) && !pkg.split('/')[0].includes('.')) return true;
  return LIBRARY_PREFIXES.some((p) => pkg === p.replace(/\.$/, '') || pkg.startsWith(p));
}

/** Profiler language → BrandLogo key and display name. */
export const LANGUAGES: Record<string, { logo: string; label: string }> = {
  jvm: { logo: 'openjdk', label: 'Java' },
  java: { logo: 'openjdk', label: 'Java' },
  go: { logo: 'go', label: 'Go' },
  python: { logo: 'python', label: 'Python' },
  dotnet: { logo: 'dotnet', label: '.NET' },
  node: { logo: 'nodedotjs', label: 'Node.js' },
  nodejs: { logo: 'nodedotjs', label: 'Node.js' },
  ruby: { logo: 'ruby', label: 'Ruby' },
  php: { logo: 'php', label: 'PHP' },
};

/** Card order: Datadog shows execution, memory and elapsed time first. */
export const CATEGORY_ORDER = ['EXECUTION', 'MEMORY', 'ELAPSED TIME', 'LOCK', 'EXCEPTIONS', 'THREADS', 'OTHER'];

/** The single service a query selects (`service:x`), if any. */
export function selectedService(query: string): string | null {
  const m = query.match(/(?:^|\s)service:("[^"]+"|[^\s()]+)(?=\s|$)/g);
  if (!m || m.length !== 1) return null;
  return m[0].trim().slice('service:'.length).replace(/"/g, '');
}
