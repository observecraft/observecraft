import { memo } from 'react';
import { BrandLogo } from './BrandLogo';

// The Kubernetes project's icons for its resource kinds (kubernetes/community, Apache-2.0; public/images/kubernetes/,
// scripts/fetch-kubernetes-icons.mjs). Kinds without one of their own (clusters, unknown kinds) show the Kubernetes logo.

/** Kind (singular, lowercase, no separators) → icon file. */
const FILES: Record<string, string> = {
  apiserver: 'control-plane',
  clusterrole: 'c-role',
  clusterrolebinding: 'crb',
  configmap: 'cm',
  controlplane: 'control-plane',
  cronjob: 'cronjob',
  customresourcedefinition: 'crd',
  daemonset: 'ds',
  deployment: 'deploy',
  endpoint: 'ep',
  endpointslice: 'ep',
  etcd: 'etcd',
  horizontalpodautoscaler: 'hpa',
  ingress: 'ing',
  job: 'job',
  limitrange: 'limits',
  namespace: 'ns',
  networkpolicy: 'netpol',
  node: 'node',
  persistentvolume: 'pv',
  persistentvolumeclaim: 'pvc',
  pod: 'pod',
  replicaset: 'rs',
  resourcequota: 'quota',
  role: 'role',
  rolebinding: 'rb',
  secret: 'secret',
  service: 'svc',
  serviceaccount: 'sa',
  statefulset: 'sts',
  storageclass: 'sc',
  volume: 'vol',
};

/** The icon file of a kind as any page spells it (Pod, pods, kube_pod, persistent_volume_claims, Ingresses), if it has one. */
export function kubernetesIconFile(kind: string): string | undefined {
  const k = kind
    .toLowerCase()
    .replace(/^kube_/, '')
    .replace(/[\s_-]/g, '');
  const singular = [k, k.replace(/ies$/, 'y'), k.replace(/(ss|sh|ch|x)es$/, '$1'), k.replace(/s$/, '')];
  for (const s of singular) if (FILES[s]) return FILES[s];
  return undefined;
}

/** A Kubernetes resource kind's icon; the Kubernetes logo for clusters and kinds without one. */
export const KubernetesIcon = memo(function KubernetesIcon({ kind, size = 16, title }: { kind?: string; size?: number; title?: string }) {
  const file = kind ? kubernetesIconFile(kind) : undefined;
  if (!file) return <BrandLogo name="kubernetes" size={size} title={title ?? 'Kubernetes'} />;
  return <img className="kube-icon" src={`/images/kubernetes/${file}.svg`} alt={title ?? ''} title={title} width={size} height={size} loading="lazy" />;
});

/** The Kubernetes logo as an icon component (section navs, cards). */
export function KubernetesLogo({ size = 16 }: { size?: number }) {
  return <BrandLogo name="kubernetes" size={size} title="Kubernetes" />;
}
