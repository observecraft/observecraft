// Detection means matching telemetry in the API's last-24-hours catalog, not installed configuration.
// Span-derived catalog entries exist even without incoming telemetry and must never count as evidence.
/** The reported metric names, indexed so that detecting an integration is a few lookups, not a scan of every name. */
export interface ReportedMetrics {
  names: Set<string>;
  /** Every dotted prefix of a reported name, with its trailing dot: `aws.ec2.cpu` → `aws.`, `aws.ec2.`. */
  prefixes: Set<string>;
}

export function reportedMetricNames(metrics: { name: string; kind: string }[]): ReportedMetrics {
  const names = new Set(metrics.filter((m) => m.kind !== 'span').map((m) => m.name));
  const prefixes = new Set<string>();
  for (const name of names) for (let i = name.indexOf('.'); i >= 0; i = name.indexOf('.', i + 1)) prefixes.add(name.slice(0, i + 1));
  return { names, prefixes };
}

const PREFIXES: Record<string, string[]> = {
  postgresql: ['postgresql.', 'postgres.'],
  mongodb: ['mongodb.'],
  redis: ['redis.'],
  kubernetes: ['k8s.', 'kubelet.', 'kubernetes.', 'kubernetes_state.'],
  nginx: ['nginx.'],
  kafka: ['kafka.'],
  rabbitmq: ['rabbitmq.'],
  coredns: ['coredns.'],
  dotnet: ['process.runtime.dotnet.', 'runtime.dotnet.', 'dotnet.'],
};

/** Declared metrics with these generic names are shared by many integrations and never identify one. */
const GENERIC = /^(http|rpc|process|system|runtime|trace)\./;

/**
 * Whether a reported name starts with one of the integration's prefixes, or is (or extends, `name.…`) one of its
 * declared metrics. `catalogPrefixes` are the integration's metric prefixes from the generated catalog (manifest
 * `metrics.prefix`); the few that don't end in a dot are matched by scanning the names.
 */
export function hasReportedMetrics(id: string, declared: { name: string }[], reported: ReportedMetrics, catalogPrefixes: string[] = []): boolean {
  for (const prefix of [...(PREFIXES[id] ?? []), ...catalogPrefixes]) {
    if (prefix.endsWith('.') ? reported.prefixes.has(prefix) : [...reported.names].some((name) => name.startsWith(prefix))) return true;
  }
  return declared.some((m) => !GENERIC.test(m.name) && (reported.names.has(m.name) || reported.prefixes.has(m.name + '.')));
}

/**
 * Installed, but none of the metrics it should send arrived (the Agent check isn't configured or fails). Only for
 * integrations with metrics of their own: logs-only ones, or ones known only by generic names, can't be judged.
 */
export function waitingForData(id: string, installed: boolean, declared: { name: string }[], reported: ReportedMetrics, catalogPrefixes: string[] = []): boolean {
  const expectsMetrics = (PREFIXES[id]?.length ?? 0) > 0 || catalogPrefixes.length > 0 || declared.some((m) => !GENERIC.test(m.name));
  return installed && expectsMetrics && !hasReportedMetrics(id, declared, reported, catalogPrefixes);
}

/** Namespaces Datadog doesn't name `aws.<namespace>.` (server aws/naming.rs PREFIXES). */
const AWS_PREFIXES: Record<string, string> = { 'AWS/RDS/Proxy': 'aws.rds.proxy.', 'ECS/ContainerInsights': 'ecs.containerinsights.' };

/** The Datadog metric prefix of a CloudWatch namespace: `AWS/EC2` → `aws.ec2.`. */
export const awsMetricPrefix = (namespace: string) =>
  AWS_PREFIXES[namespace] ??
  `aws.${namespace
    .replace(/^\/?aws\//i, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')}.`;
