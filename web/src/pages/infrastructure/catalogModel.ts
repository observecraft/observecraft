// Labels and links of the resource catalog (Infrastructure → Catalog).
import type { CatalogCategory, CatalogResource, CatalogType } from '../../api/catalog/client';

export const TYPE_LABELS: Record<CatalogType, string> = {
  host: 'Host',
  kube_cluster: 'Kubernetes Cluster',
  kube_node: 'Kubernetes Node',
  kube_namespace: 'Kubernetes Namespace',
  kube_deployment: 'Kubernetes Deployment',
  kube_statefulset: 'Kubernetes StatefulSet',
  kube_daemonset: 'Kubernetes DaemonSet',
  kube_cronjob: 'Kubernetes CronJob',
  kube_job: 'Kubernetes Job',
  kube_pod: 'Kubernetes Pod',
  kube_service: 'Kubernetes Service',
  kube_ingress: 'Kubernetes Ingress',
  kube_persistent_volume_claim: 'Kubernetes PVC',
  kube_persistent_volume: 'Kubernetes PV',
  container: 'Container',
  ecs_cluster: 'ECS Cluster',
  ecs_service: 'ECS Service',
  ecs_task: 'ECS Task',
};

export const CATEGORY_LABELS: Record<CatalogCategory, string> = {
  compute: 'Compute',
  containers: 'Containers',
  kubernetes: 'Kubernetes',
  ecs: 'ECS',
};

const FACET_LABELS: Record<string, string> = {
  resource_category: 'Resource Category',
  resource_type: 'Resource Type',
  cloud: 'Cloud Provider',
  account: 'Account',
  region: 'Region',
  cluster: 'Cluster',
  namespace: 'Namespace',
  env: 'Environment',
  service: 'Service',
  team: 'Team',
};

export function facetLabel(key: string): string {
  return FACET_LABELS[key] ?? key;
}

/** The page that shows the resource: the host panel, or its Kubernetes / ECS Explorer panel. */
export function catalogLink(r: Pick<CatalogResource, 'resourceType' | 'id' | 'name' | 'kubeKind'>): string | null {
  if (r.resourceType === 'host') return `/infrastructure?${new URLSearchParams({ host: r.name })}`;
  if (!r.kubeKind) return null;
  return `/orchestration/explorer?${new URLSearchParams({ resource: r.kubeKind, panel: r.id })}`;
}
