// Metric facets shared by the Catalog and the Overview: which product a metric comes from and its type group.
import type { MetricCatalogEntry } from '../../api/dashboards/types';
import type { MetricRule } from '../../api/metricRules/client';
import { globMatch } from '../../lib/metricRules';

export type Origin = 'Agent' | 'Agent integrations' | 'Cloud integrations' | 'OpenTelemetry' | 'APM' | 'Logs' | 'Events' | 'Derived' | 'Custom' | 'ObserveCraft';
export const ORIGINS: Origin[] = ['Agent', 'Agent integrations', 'Cloud integrations', 'OpenTelemetry', 'APM', 'Logs', 'Events', 'Derived', 'Custom', 'ObserveCraft'];
/** What the platform and its integrations report; everything else is custom (sent by your own code or generated). */
export const STANDARD_ORIGINS: Origin[] = ['Agent', 'Agent integrations', 'Cloud integrations', 'APM', 'ObserveCraft'];
export const isStandard = (o: Origin) => STANDARD_ORIGINS.includes(o);

/** What the Datadog Agent itself reports (core checks), as opposed to its integrations. */
const AGENT_CORE = /^(system|datadog|process|container|containerd|cri|docker|ntp|disk|network|io|uptime|load|service_check|check_run|dd)\./;
/** Namespaces of the Agent's integrations (integrations-core checks). */
const AGENT_INTEGRATIONS = new RegExp(
  `^(${[
    'kubernetes',
    'kubernetes_state',
    'kube_apiserver',
    'kube_controller_manager',
    'kube_scheduler',
    'kube_proxy',
    'kubelet',
    'etcd',
    'coredns',
    'cilium',
    'calico',
    'istio',
    'envoy',
    'linkerd',
    'karpenter',
    'argocd',
    'argo_rollouts',
    'cert_manager',
    'nginx',
    'nginx_ingress',
    'haproxy',
    'traefik',
    'redis',
    'redisdb',
    'postgresql',
    'mysql',
    'mongodb',
    'cassandra',
    'elasticsearch',
    'opensearch',
    'kafka',
    'kafka_consumer',
    'rabbitmq',
    'zookeeper',
    'consul',
    'vault',
    'jvm',
    'tomcat',
    'jmx',
    'gunicorn',
    'php_fpm',
    'apache',
    'iis',
    'sqlserver',
    'oracle',
    'clickhouse',
    'memcache',
    'nats',
    'spark',
    'airflow',
    'gpu',
    'nvidia',
    'dcgm',
    'snmp',
    'http',
    'tls',
    'dns',
    'tcp',
    'win32_event_log',
    'windows_service',
    'helm',
    'falco',
    'vsphere',
    'openmetrics',
    'prometheus',
    'gitlab',
    'jenkins',
    'teamcity',
    'cri_o',
    'crio',
    'oom_kill',
    'tcp_queue_length',
    'kafka_broker',
    'aerospike',
    'couchbase',
    'druid',
    'ceph',
  ].join('|')})\\.`,
);
/** Cloud provider integrations (crawled, no Agent). */
const CLOUD = /^(aws|azure|gcp|gce|cloudflare|fastly|alibaba|oci)\./;
/** OpenTelemetry semantic-convention namespaces (`k8s.pod.cpu.usage`, `http.server.request.duration`, …). */
const OTEL =
  /^(k8s|rpc|db|messaging|faas|process\.runtime|runtime|go|dotnet|nodejs|v8js|otelcol|hw|gen_ai|jvm\.memory\.used)\.|^http\.(server|client)\.|^system\.cpu\.utilization|_(total|bucket|seconds|bytes)$/;

/** The product a metric comes from, from its name and kind (the intake keeps names as sent). */
export function metricOrigin(m: { name: string; kind?: string }, logMetrics: Set<string>, eventMetrics: Set<string> = new Set()): Origin {
  const base = m.name.replace(/\.(count|sum|avg|min|max|p50|p75|p90|p95|p99)$/, '');
  if (m.kind === 'derived') return 'Derived';
  if (m.kind === 'span' || m.name.startsWith('trace.')) return 'APM';
  if (logMetrics.has(m.name) || logMetrics.has(base)) return 'Logs';
  if (eventMetrics.has(m.name) || eventMetrics.has(base)) return 'Events';
  if (m.name.startsWith('observecraft.')) return 'ObserveCraft';
  if (OTEL.test(m.name)) return 'OpenTelemetry';
  if (CLOUD.test(m.name)) return 'Cloud integrations';
  if (AGENT_CORE.test(m.name)) return 'Agent';
  if (AGENT_INTEGRATIONS.test(m.name)) return 'Agent integrations';
  return 'Custom';
}

export type TypeGroup = 'Counts, Rates, Gauges' | 'Distributions' | 'Derived' | 'From traces';
export const TYPE_GROUPS: TypeGroup[] = ['Counts, Rates, Gauges', 'Distributions', 'Derived', 'From traces'];

export function typeGroup(kind: string | undefined): TypeGroup {
  if (kind === 'derived') return 'Derived';
  if (kind === 'span') return 'From traces';
  if (kind === 'histogram' || kind === 'exponential_histogram' || kind === 'summary') return 'Distributions';
  return 'Counts, Rates, Gauges';
}

/** The distribution a catalog entry is a statistic of (`latency.p95` → `latency`). */
export const distributionBase = (m: MetricCatalogEntry) => (typeGroup(m.kind) === 'Distributions' ? m.name.replace(/\.[^.]+$/, '') : m.name);

/** The tags queries may use: those of the enabled indexing rules that match it, else all. */
export function queryableTags(name: string, all: string[], rules: MetricRule[]): string[] {
  const allowed = rules.filter((r) => r.enabled && r.kind === 'indexing' && globMatch(r.metricPattern, name));
  if (!allowed.length) return all;
  const keep = new Set(allowed.flatMap((r) => r.tags));
  return all.filter((t) => keep.has(t));
}

/** The latest change to a rule that applies to the metric (epoch ms), if any. */
export function lastTagConfiguration(name: string, rules: MetricRule[]): number | undefined {
  const times = rules.filter((r) => globMatch(r.metricPattern, name)).map((r) => r.updatedAt);
  return times.length ? Math.max(...times) : undefined;
}

/** 1234 → "1.23K", as the catalog's counters read. */
export function compactCount(n: number): string {
  for (const [limit, suffix] of [
    [1e9, 'B'],
    [1e6, 'M'],
    [1e3, 'K'],
  ] as const) {
    if (Math.abs(n) >= limit) return `${(n / limit).toLocaleString('en-US', { maximumFractionDigits: n / limit >= 100 ? 0 : n / limit >= 10 ? 1 : 2 })}${suffix}`;
  }
  return n.toLocaleString('en-US');
}

/** A rows → CSV download. */
export function downloadCsv(name: string, header: string[], rows: (string | number)[][]) {
  const cell = (v: string | number) => (/[",\n]/.test(String(v)) ? `"${String(v).replace(/"/g, '""')}"` : String(v));
  const text = [header, ...rows].map((r) => r.map(cell).join(',')).join('\n');
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([text], { type: 'text/csv' }));
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
}
