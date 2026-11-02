// Kubernetes → Autoscaling: the manifests and commands users apply to their clusters. The Datadog
// Cluster Agent does the scaling: DatadogPodAutoscalerClusterProfile templates (applied to workloads
// labelled `autoscaling.datadoghq.com/profile`), DatadogPodAutoscalers (one per workload; the
// `custom-recommender` annotation points it at ObserveCraft's recommender), and plain requests for
// vertical recommendations (delivered by Datadog's Remote Configuration, which ObserveCraft doesn't run).

export const PROFILE_LABEL = 'autoscaling.datadoghq.com/profile';
export const RECOMMENDER_ANNOTATION = 'autoscaling.datadoghq.com/custom-recommender';
const API_VERSION = 'datadoghq.com/v1alpha2';

export type Dimensions = 'both' | 'vertical' | 'horizontal';

/** The Create Profile form. */
export interface ProfileForm {
  dimensions: Dimensions;
  /** CPU utilization target, percent of requests. */
  cpuTarget: number;
  /** Container whose CPU is measured; empty: the whole pod. */
  container: string;
  minReplicas: number;
  maxReplicas: number;
  scaleUpPercent: number;
  scaleUpPeriodSeconds: number;
  scaleUpWindowSeconds: number;
  scaleDownPercent: number;
  scaleDownPeriodSeconds: number;
  scaleDownWindowSeconds: number;
}

export const DEFAULT_PROFILE: ProfileForm = {
  dimensions: 'both',
  cpuTarget: 75,
  container: '',
  minReplicas: 2,
  maxReplicas: 10,
  scaleUpPercent: 50,
  scaleUpPeriodSeconds: 120,
  scaleUpWindowSeconds: 600,
  scaleDownPercent: 20,
  scaleDownPeriodSeconds: 1200,
  scaleDownWindowSeconds: 600,
};

type Json = string | number | boolean | null | Json[] | { [key: string]: Json };
export type ProfileTemplate = { [key: string]: Json };

/** `spec.template` of a profile (also the `spec` of a DatadogPodAutoscaler, without its target). */
export function profileTemplate(f: ProfileForm): ProfileTemplate {
  const horizontal = f.dimensions !== 'vertical';
  const vertical = f.dimensions !== 'horizontal';
  const policy = (percent: number, period: number, window: number) => ({ strategy: 'Max', stabilizationWindowSeconds: window, rules: [{ type: 'Percent', value: percent, periodSeconds: period }] });
  const value = { type: 'Utilization', utilization: f.cpuTarget };
  const constraints: { [key: string]: Json } = horizontal ? { minReplicas: f.minReplicas, maxReplicas: f.maxReplicas } : {};
  if (vertical) constraints.containers = [{ name: '*', enabled: true }];
  const template: ProfileTemplate = {
    applyPolicy: {
      mode: 'Apply',
      scaleUp: horizontal ? policy(f.scaleUpPercent, f.scaleUpPeriodSeconds, f.scaleUpWindowSeconds) : { strategy: 'Disabled' },
      scaleDown: horizontal ? policy(f.scaleDownPercent, f.scaleDownPeriodSeconds, f.scaleDownWindowSeconds) : { strategy: 'Disabled' },
      update: { strategy: vertical ? 'Auto' : 'Disabled' },
    },
    constraints,
  };
  if (horizontal) {
    template.objectives = [
      f.container ? { type: 'ContainerResource', containerResource: { name: 'cpu', container: f.container, value } } : { type: 'PodResource', podResource: { name: 'cpu', value } },
    ];
  }
  return template;
}

// ---------------------------------------------------------------- YAML

const PLAIN = /^[A-Za-z_/][A-Za-z0-9_./-]*$/;
const RESERVED = /^(true|false|yes|no|on|off|null|y|n|~)$/i;

function scalar(v: Json): string {
  if (typeof v === 'string') return PLAIN.test(v) && !RESERVED.test(v) ? v : JSON.stringify(v);
  return String(v);
}

function emit(v: Json, indent: string): string[] {
  if (Array.isArray(v)) {
    if (!v.length) return [`${indent}[]`];
    return v.flatMap((item) => {
      const [first, ...rest] = emit(item, `${indent}  `);
      return [`${indent}- ${first.trimStart()}`, ...rest];
    });
  }
  if (v && typeof v === 'object') {
    const entries = Object.entries(v);
    if (!entries.length) return [`${indent}{}`];
    return entries.flatMap(([k, x]) => {
      const key = `${indent}${scalar(k)}:`;
      if (x && typeof x === 'object' && (Array.isArray(x) ? x.length : Object.keys(x).length)) return [key, ...emit(x, `${indent}  `)];
      return [`${key} ${Array.isArray(x) ? '[]' : x && typeof x === 'object' ? '{}' : scalar(x)}`];
    });
  }
  return [`${indent}${scalar(v)}`];
}

export const toYaml = (v: Json) => emit(v, '').join('\n') + '\n';

// ---------------------------------------------------------------- manifests

export interface WorkloadRef {
  namespace: string;
  /** Deployment or StatefulSet. */
  kind: string;
  name: string;
}

export function profileManifest(name: string, template: ProfileTemplate): string {
  return toYaml({ apiVersion: API_VERSION, kind: 'DatadogPodAutoscalerClusterProfile', metadata: { name }, spec: { template } });
}

/** The endpoint for the `custom-recommender` annotation (the intake, with an ingest key). */
export function recommenderEndpoint(intakeUrl: string): string {
  return `${intakeUrl.trim().replace(/\/+$/, '')}/api/v2/autoscaling/recommend?api_key=<INGEST_API_KEY>`;
}

/** One workload's DatadogPodAutoscaler; with `endpoint`, the Cluster Agent asks ObserveCraft for replicas. */
export function podAutoscalerManifest(w: WorkloadRef, template: ProfileTemplate, endpoint?: string): string {
  const metadata: { [key: string]: Json } = { name: w.name, namespace: w.namespace };
  if (endpoint) metadata.annotations = { [RECOMMENDER_ANNOTATION]: JSON.stringify({ endpoint }) };
  return toYaml({ apiVersion: API_VERSION, kind: 'DatadogPodAutoscaler', metadata, spec: { targetRef: { apiVersion: 'apps/v1', kind: w.kind, name: w.name }, owner: 'Local', ...template } });
}

// ---------------------------------------------------------------- commands

/** 0.231 cores → `231m`. */
export function cores(v: number): string {
  const milli = Math.max(1, Math.ceil(v * 1000 - 1e-6));
  return milli % 1000 === 0 ? String(milli / 1000) : `${milli}m`;
}

/** Bytes → `589Mi`. */
export const mebibytes = (v: number) => `${Math.max(1, Math.ceil(v / (1024 * 1024) - 1e-6))}Mi`;

export function labelCommand(w: WorkloadRef, profile: string): string {
  return `kubectl -n ${w.namespace} label ${w.kind.toLowerCase()} ${w.name} ${PROFILE_LABEL}=${profile} --overwrite`;
}

export interface ContainerRequests {
  name: string;
  recommendedCpu?: number | null;
  recommendedMemory?: number | null;
}

/** `kubectl set resources` with the recommended requests of every container that has one. */
export function setResourcesCommand(w: WorkloadRef, containers: ContainerRequests[]): string {
  return containers
    .filter((c) => c.recommendedCpu != null || c.recommendedMemory != null)
    .map((c) => {
      const requests = [c.recommendedCpu != null ? `cpu=${cores(c.recommendedCpu)}` : '', c.recommendedMemory != null ? `memory=${mebibytes(c.recommendedMemory)}` : ''].filter(Boolean).join(',');
      return `kubectl -n ${w.namespace} set resources ${w.kind.toLowerCase()} ${w.name} -c ${c.name} --requests=${requests}`;
    })
    .join('\n');
}

export function uninstallCommands(autoscalers: WorkloadRef[]): string[] {
  return autoscalers.map((a) => `kubectl -n ${a.namespace} delete ${a.kind.toLowerCase()} ${a.name}`);
}

/** The Create Profile form of a template (a saved profile opened again); missing parts take the defaults. */
export function formFromTemplate(t: Record<string, unknown>): ProfileForm {
  const get = (path: (string | number)[]): unknown => path.reduce<unknown>((v, k) => (v && typeof v === 'object' ? (v as Record<string | number, unknown>)[k] : undefined), t);
  const num = (path: (string | number)[], fallback: number) => (typeof get(path) === 'number' ? (get(path) as number) : fallback);
  const objective = get(['objectives', 0]) as Record<string, unknown> | undefined;
  const horizontal = Boolean(objective) && get(['applyPolicy', 'scaleUp', 'strategy']) !== 'Disabled';
  const vertical = get(['applyPolicy', 'update', 'strategy']) !== 'Disabled';
  const resource = objective?.type === 'ContainerResource' ? 'containerResource' : 'podResource';
  const d = DEFAULT_PROFILE;
  return {
    dimensions: horizontal && vertical ? 'both' : horizontal ? 'horizontal' : 'vertical',
    cpuTarget: num(['objectives', 0, resource, 'value', 'utilization'], d.cpuTarget),
    container: typeof get(['objectives', 0, 'containerResource', 'container']) === 'string' ? (get(['objectives', 0, 'containerResource', 'container']) as string) : '',
    minReplicas: num(['constraints', 'minReplicas'], d.minReplicas),
    maxReplicas: num(['constraints', 'maxReplicas'], d.maxReplicas),
    scaleUpPercent: num(['applyPolicy', 'scaleUp', 'rules', 0, 'value'], d.scaleUpPercent),
    scaleUpPeriodSeconds: num(['applyPolicy', 'scaleUp', 'rules', 0, 'periodSeconds'], d.scaleUpPeriodSeconds),
    scaleUpWindowSeconds: num(['applyPolicy', 'scaleUp', 'stabilizationWindowSeconds'], d.scaleUpWindowSeconds),
    scaleDownPercent: num(['applyPolicy', 'scaleDown', 'rules', 0, 'value'], d.scaleDownPercent),
    scaleDownPeriodSeconds: num(['applyPolicy', 'scaleDown', 'rules', 0, 'periodSeconds'], d.scaleDownPeriodSeconds),
    scaleDownWindowSeconds: num(['applyPolicy', 'scaleDown', 'stabilizationWindowSeconds'], d.scaleDownWindowSeconds),
  };
}
