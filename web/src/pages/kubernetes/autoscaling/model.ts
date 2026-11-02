// Kubernetes → Autoscaling: the pages' filters and totals over the server's overview.
import type { AutoscalingOverview, AutoscalingSummary, ClusterScaling, KubeAutoscaler, WorkloadRecommendation } from '../../../api/kubernetes/client';

type Filterable = Pick<AutoscalingOverview, 'clusters' | 'workloads' | 'autoscalers'>;

/** The overview's clusters, workloads and autoscalers of one cluster and namespace (empty: all). */
export function filterOverview<T extends Filterable>(o: T, cluster: string, namespace: string): { clusters: ClusterScaling[]; workloads: WorkloadRecommendation[]; autoscalers: KubeAutoscaler[] } {
  const inScope = (c: string, ns?: string) => (!cluster || c === cluster) && (!namespace || ns === namespace);
  return {
    clusters: o.clusters.filter((c) => !cluster || c.cluster === cluster),
    workloads: o.workloads.filter((w) => inScope(w.cluster, w.namespace)),
    autoscalers: o.autoscalers.filter((a) => inScope(a.cluster, a.namespace)),
  };
}

/** Totals of the filtered page. Cluster idle capacity belongs to no namespace, so a namespace filter leaves it out. */
export function summarize(clusters: ClusterScaling[], workloads: WorkloadRecommendation[], namespaceFiltered: boolean): AutoscalingSummary {
  const sum = <T>(xs: T[], f: (x: T) => number) => xs.reduce((s, x) => s + f(x), 0);
  return {
    usageCost: sum(workloads, (w) => w.usageCost),
    workloadIdleCost: sum(workloads, (w) => w.idleCost),
    clusterIdleCost: namespaceFiltered ? 0 : sum(clusters, (c) => c.clusterIdleCost),
    workloadSavings: sum(workloads, (w) => w.projectedSavings),
    clusterSavings: namespaceFiltered ? 0 : sum(clusters, (c) => c.projectedClusterSavings),
    workloads: workloads.length,
    unscaledWorkloads: workloads.filter((w) => !w.autoscaler).length,
    clusters: clusters.length,
    unscaledClusters: clusters.filter((c) => c.autoscaling === 'none').length,
  };
}

export type WorkloadStatus = 'ready' | 'not_ready' | 'scaled';

export function workloadStatus(w: WorkloadRecommendation): WorkloadStatus {
  if (w.autoscaler) return 'scaled';
  return w.notReady.length ? 'not_ready' : 'ready';
}

export interface FacetValue {
  value: string;
  count: number;
}

/** The values of one facet and how many items have each, the most common first. */
export function facetValues<T>(items: T[], key: (item: T) => string | null | undefined): FacetValue[] {
  const counts = new Map<string, number>();
  for (const i of items) {
    const v = key(i);
    if (v) counts.set(v, (counts.get(v) ?? 0) + 1);
  }
  return [...counts.entries()].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value));
}

/** Items matching every facet that has a selection (any of its selected values). */
export function applyFacets<T>(items: T[], keys: Record<string, (item: T) => string | null | undefined>, selected: Record<string, string[]>): T[] {
  return items.filter((i) => Object.entries(selected).every(([k, values]) => !values.length || values.includes(keys[k]?.(i) ?? '')));
}
