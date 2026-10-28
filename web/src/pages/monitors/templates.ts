// Built-in monitor templates: a query, thresholds and a message to start from.
import type { MonitorOptions, MonitorType } from '../../api/monitors/client';
import { KUBERNETES_TEMPLATES } from './kubernetesTemplates.generated';

export interface MonitorTemplate {
  id: string;
  /** Integration label shown above the title (SYSTEM, APM, …). */
  source: string;
  title: string;
  description: string;
  type: MonitorType;
  name: string;
  query: string;
  options: MonitorOptions;
  message: string;
  /** Words that match a plain-language request in "Create with Assist". */
  keywords: string[];
}

const threshold = (over: Partial<MonitorOptions>, critical: number, warning?: number): MonitorOptions => ({
  detection: 'threshold',
  timeAggregation: 'avg',
  windowMinutes: 5,
  comparator: 'above',
  thresholds: {
    critical,
    warning: warning ?? null,
    criticalRecovery: null,
    warningRecovery: null,
  },
  notifyNoData: false,
  noDataMinutes: 10,
  changeMode: null,
  changeMinutes: null,
  ...over,
});

export const TEMPLATES: MonitorTemplate[] = [
  {
    id: 'cpu-high',
    source: 'SYSTEM',
    title: 'CPU usage is high',
    description: 'Alert when a host spends most of its CPU time in user space.',
    type: 'metric',
    name: 'CPU usage is high on {{host.name}}',
    query: 'avg:system.cpu.user{*} by {host}',
    options: threshold({}, 90, 80),
    message:
      '{{#is_alert}}CPU is at {{value}}% on {{host.name}}.{{/is_alert}}\n{{#is_warning}}CPU is climbing on {{host.name}} ({{value}}%).{{/is_warning}}\n{{#is_recovery}}CPU is back to normal on {{host.name}}.{{/is_recovery}}',
    keywords: ['cpu', 'processor', 'compute', 'host', 'core', 'app', 'performance'],
  },
  {
    id: 'mem-high',
    source: 'SYSTEM',
    title: 'Memory consumption is high',
    description: 'Alert when a host is running out of memory.',
    type: 'metric',
    name: 'Memory usage is high on {{host.name}}',
    // The Agent's usable share (0–1) as % used; a formula, as Datadog's memory monitor is.
    query: '100 - avg:system.mem.pct_usable{*} by {host} * 100',
    options: threshold({ windowMinutes: 10 }, 90, 80),
    message: 'Memory is at {{value}}% on {{host.name}}.',
    keywords: ['memory', 'ram', 'oom', 'mem'],
  },
  {
    id: 'disk-low',
    source: 'SYSTEM',
    title: 'Disk space is running low',
    description: 'Alert before a volume fills up.',
    type: 'metric',
    name: 'Disk space is running low on {{host.name}}',
    query: 'max:system.disk.in_use{*} by {host}',
    options: threshold(
      {
        timeAggregation: 'max',
        windowMinutes: 15,
        notifyNoData: true,
        noDataMinutes: 30,
      },
      90,
      80,
    ),
    message: '{{host.name}} is {{value}}% full.',
    keywords: ['disk', 'storage', 'volume', 'space', 'filesystem'],
  },
  {
    id: 'load-high',
    source: 'SYSTEM',
    title: 'Load average is high',
    description: 'Alert when more processes are waiting for CPU than the host can run.',
    type: 'metric',
    name: 'Load average is high on {{host.name}}',
    query: 'avg:system.load.5{*} by {host}',
    options: threshold({ windowMinutes: 10 }, 8, 6),
    message: 'Load is {{value}} on {{host.name}}.',
    keywords: ['load', 'saturation', 'run queue'],
  },
  {
    id: 'net-in',
    source: 'NETWORK',
    title: 'Inbound traffic is unusually high',
    description: 'Alert on sustained inbound network throughput.',
    type: 'metric',
    name: 'High inbound network traffic on {{host.name}}',
    query: 'avg:system.net.bytes_rcvd{*} by {host}',
    options: threshold({ windowMinutes: 10 }, 100_000_000, 50_000_000),
    message: '{{host.name}} is receiving {{value}} B/s.',
    keywords: ['network', 'bandwidth', 'traffic', 'bytes', 'ddos'],
  },
  {
    id: 'apm-errors',
    source: 'APM',
    title: 'Error rate is high',
    description: 'Alert when a service fails a growing share of its requests.',
    type: 'apm',
    name: 'Error rate is high on {{service.name}}',
    query: 'avg:trace.error_rate{env:prod} by {service}',
    options: threshold({ notifyNoData: false }, 5, 2),
    message:
      '{{#is_alert}}{{service.name}} is failing {{value}}% of requests.{{/is_alert}}\n{{#is_warning}}{{service.name}} errors at {{value}}%.{{/is_warning}}\n{{#is_recovery}}{{service.name}} error rate recovered.{{/is_recovery}}',
    keywords: ['error', 'errors', 'failure', 'failing', '5xx', 'exception', 'service', 'app', 'application', 'api'],
  },
  {
    id: 'apm-latency',
    source: 'APM',
    title: 'p95 latency is high',
    description: 'Alert when requests slow down for most users.',
    type: 'apm',
    name: 'p95 latency is high on {{service.name}}',
    query: 'avg:trace.latency.p95{env:prod} by {service}',
    options: threshold({ windowMinutes: 10 }, 500, 300),
    message: '{{service.name}} p95 latency is {{value}} ms.',
    keywords: ['latency', 'slow', 'response time', 'performance', 'p95', 'p99', 'duration', 'app', 'api'],
  },
  {
    id: 'apm-traffic-drop',
    source: 'APM',
    title: 'Traffic dropped suddenly',
    description: 'Alert when a service receives far fewer requests than an hour ago.',
    type: 'apm',
    name: 'Request volume dropped on {{service.name}}',
    query: 'sum:trace.requests{env:prod} by {service}',
    options: threshold(
      {
        detection: 'change',
        changeMode: 'pct_change',
        changeMinutes: 60,
        comparator: 'below',
      },
      -50,
      -30,
    ),
    message: '{{service.name}} traffic changed by {{value}}% compared with an hour ago.',
    keywords: ['traffic', 'requests', 'throughput', 'volume', 'drop', 'outage', 'down'],
  },
  {
    id: 'apm-no-traffic',
    source: 'APM',
    title: 'Service stopped receiving requests',
    description: 'Alert when a service goes quiet.',
    type: 'apm',
    name: '{{service.name}} has no traffic',
    query: 'sum:trace.requests{env:prod} by {service}',
    options: threshold(
      {
        comparator: 'below',
        windowMinutes: 10,
        notifyNoData: true,
        noDataMinutes: 10,
      },
      0.01,
    ),
    message: '{{service.name}} stopped receiving requests.',
    keywords: ['availability', 'down', 'outage', 'uptime', 'heartbeat', 'dead'],
  },
  // integrations-core's recommended monitors (generated, BSD-3-Clause).
  ...KUBERNETES_TEMPLATES,
];

/** Picks templates for a plain-language request ("I want to monitor my checkout latency"). */
export function suggestTemplates(text: string): MonitorTemplate[] {
  const t = text.toLowerCase();
  return TEMPLATES.map((tpl) => ({
    tpl,
    score: tpl.keywords.filter((k) => t.includes(k)).length,
  }))
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score)
    .map((x) => x.tpl);
}
