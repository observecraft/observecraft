// Pod usage series under either naming scheme: OpenTelemetry (k8s.pod.*) or the Datadog Agent's kubelet check.
import type { MetricSeries } from '../types';

/** The Datadog Agent's usage metric per resource, and the factor to the OpenTelemetry unit (cores, bytes). */
export const DATADOG_USAGE: Record<'cpu' | 'memory', { metric: string; scale: number }> = {
  cpu: { metric: 'kubernetes.cpu.usage.total', scale: 1e-9 },
  memory: { metric: 'kubernetes.memory.working_set', scale: 1 },
};

/** The OpenTelemetry series when there are any, else the Datadog ones in the same unit. */
export function usageSeries(res: 'cpu' | 'memory', otel: MetricSeries[], datadog: MetricSeries[]): MetricSeries[] {
  if (otel.length) return otel;
  const { scale } = DATADOG_USAGE[res];
  return datadog.map((s) => ({ ...s, points: s.points.map(([t, v]) => [t, v * scale]) }));
}

/**
 * A group's series by name; with a single cluster, also the series whose metrics carry no cluster
 * (`kube_cluster_name:N/A`: a Datadog Agent without a cluster name, or data from before host tags).
 */
export function groupSeries(series: Map<string, MetricSeries>, name: string, singleCluster: boolean): MetricSeries | undefined {
  return series.get(name) ?? (singleCluster ? series.get(name.replace(/kube_cluster_name:[^,]*/, 'kube_cluster_name:N/A')) : undefined);
}
