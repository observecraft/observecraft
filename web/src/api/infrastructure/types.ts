import type { components } from '../generated/schema';
// Infrastructure (Hosts) wire contract. Hosts are reported by collectors (ObserveCraft
// collector = OTel Collector + Vector bundle, or a plain OpenTelemetry Collector).
import type { MetricSeries, TimeRange } from '../types';

export type Cloud = 'aws' | 'azure' | 'gcp' | 'oracle' | 'alibaba' | 'none';
export type HostOs = 'linux' | 'windows' | 'darwin';
export type CollectorKind = 'observecraft' | 'otel';
export type Software = 'kubernetes' | 'docker';

export interface MonitorCounts {
  alert: number;
  warn: number;
  ok: number;
}

export interface HostSummary {
  name: string;
  cloud: Cloud;
  os: HostOs;
  /** Distro shown in place of the generic OS icon when known ("ubuntu"). */
  distro?: string;
  collectors: CollectorKind[];
  software: Software[];
  env: string | null;
  service: string | null;
  team: string | null;
  gpu: boolean;
  monitors: MonitorCounts;
  cpuPct: number;
  memPct: number;
  /** APM error spans from this host in range; null when APM isn't enabled. */
  errorSpans: number | null;
}

export type HostSortField = 'name' | 'cpu' | 'mem' | 'env' | 'monitors';

export interface MetricFilter {
  metric: string; // "system.cpu.user"
  min?: number;
  max?: number;
}

export interface HostSearchRequest {
  range: TimeRange;
  /** Tag query: `cloud:aws os:linux env:prod team:storefront gpu:true collector:otel` + free text on name. */
  query: string;
  metricFilters: MetricFilter[];
  groupBy: string | null; // tag key
  sort: { field: HostSortField; dir: 'asc' | 'desc' };
  offset: number;
  limit: number;
}

export interface HostSearchPage {
  hosts: (HostSummary & { group?: string })[];
  total: number;
  /** When grouped: every group in the full result, in display order. */
  groups?: { key: string; count: number }[];
}

export interface TriggeredMonitor {
  id: string;
  name: string;
  status: 'alert' | 'warn';
  query: string;
  threshold: number;
  series: [number, number][];
}

export interface HostIntegration {
  name: string; // "nginx_ingress_controller"
  status: 'ok' | 'broken';
  metrics: string[];
}

export interface HostDetail extends HostSummary {
  account: string | null;
  region: string | null;
  hostType: string; // "EC2 Instance", "Other Host"
  collectorVersion: string;
  uptimeSec: number;
  ntpOffsetMs: number;
  tags: Record<string, string>;
  products: ('apm' | 'infra' | 'logs' | 'processes')[];
  triggeredMonitors: TriggeredMonitor[];
  integrations: HostIntegration[];
  kubernetes: { cluster: string; criName: string; criVersion: string; kubeletVersion: string } | null;
  /** Collector config currently applied to this host. */
  collectorConfig: string;
  pipelines: { name: string; receivers: string[]; processors: string[]; exporters: string[] }[];
}

export interface HostMetrics {
  cpu: MetricSeries[];
  load: MetricSeries[];
  memory: MetricSeries[];
  processMemory: { name: string; value: number }[];
}

/** A pod on a host (from the Kubernetes collector). */
export type Pod = components['schemas']['HostPod'];

/** A container of a pod on a host. */
export type Container = components['schemas']['HostContainer'];

export interface Process {
  pid: number;
  command: string;
  user: string;
  cpuPct: number;
  rssBytes: number;
}

export interface PageRequest {
  host: string;
  query: string;
  offset: number;
  limit: number;
}

export interface Paged<T> {
  items: T[];
  total: number;
}

export interface ProcessPage extends Paged<Process> {
  /** False when process collection isn't enabled on this host's collector. */
  enabled: boolean;
}
