// Detection means matching telemetry in the API's last-24-hours catalog, not installed configuration.
// Span-derived catalog entries exist even without incoming telemetry and must never count as evidence.
export function reportedMetricNames(metrics: { name: string; kind: string }[]): Set<string> {
  return new Set(metrics.filter((m) => m.kind !== 'span').map((m) => m.name));
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

export function hasReportedMetrics(id: string, declared: { name: string }[], names: Set<string>): boolean {
  const prefixes = PREFIXES[id] ?? [];
  for (const name of names) {
    if (prefixes.some((prefix) => name.startsWith(prefix))) return true;
    if (declared.some((m) => !/^(http|rpc|process|system|runtime|trace)\./.test(m.name) && (name === m.name || name.startsWith(m.name + '.')))) return true;
  }
  return false;
}
