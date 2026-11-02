// Kubernetes Explorer data access. In "live" mode objects come from the Rust API (the cluster
// collector's objects, events and kubelet metrics in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/orchestration/search          searchKubernetes
//   GET  /api/v1/orchestration/resources/:uid  getKubeResource
//   GET  /api/v1/kubernetes/clusters           listKubeClusters (Integrations → Kubernetes)
//   GET  /api/v1/kubernetes/autoscaling        getAutoscaling (Kubernetes → Autoscaling)
//   PUT  /api/v1/kubernetes/autoscaling/settings, POST/PUT/DELETE …/profiles[/:id]
//   GET  /api/v1/kubernetes/remediation        listWorkloadProblems (Kubernetes → Remediation)
//   GET  /api/v1/kubernetes/rollouts           listRollouts (Kubernetes → Live Rollouts)
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/kubernetes';
import * as autoscalingMock from '../mock/kubernetesAutoscaling';

const live = API_MODE === 'live';

export type KubeKind = components['schemas']['KubeKind'];
export type KubeHealth = components['schemas']['KubeHealth'];
export type KubeResource = components['schemas']['KubeResource'];
export type KubeUtilization = components['schemas']['KubeUtilization'];
export type KubeContainer = components['schemas']['KubeContainer'];
export type KubeSearchRequest = components['schemas']['KubeSearchRequest'];
export type KubeSearchPage = components['schemas']['KubeSearchPage'];
export type KubeDetail = components['schemas']['KubeDetail'];
export type KubeEvent = components['schemas']['KubeEvent'];
export type KubeFacet = components['schemas']['KubeFacet'];
export type KubeGroup = components['schemas']['KubeGroup'];
export type KubeClusterSummary = components['schemas']['KubeClusterSummary'];
export type KubeObjectSource = components['schemas']['KubeObjectSource'];
/** A profile's `spec.template` (free-form JSON in the OpenAPI document). */
export type ProfileTemplateJson = Record<string, unknown>;
type WithTemplate<T> = Omit<T, 'template'> & { template: ProfileTemplateJson };
export type AutoscalingOverview = Omit<components['schemas']['AutoscalingOverview'], 'profiles'> & { profiles: AutoscalingProfile[] };
export type AutoscalingSettings = components['schemas']['AutoscalingSettings'];
export type AutoscalingSummary = components['schemas']['AutoscalingSummary'];
export type AutoscalingProfile = WithTemplate<components['schemas']['AutoscalingProfile']>;
export type WorkloadRecommendation = components['schemas']['WorkloadRecommendation'];
export type ContainerRecommendation = components['schemas']['ContainerRecommendation'];
export type KubeAutoscaler = components['schemas']['KubeAutoscaler'];
export type ClusterScaling = components['schemas']['ClusterScaling'];
export type StoredRecommendation = components['schemas']['StoredRecommendation'];
export type ProfileInput = WithTemplate<components['schemas']['ProfileInput']>;
export type SavedProfile = WithTemplate<components['schemas']['SavedProfileView']>;
export type WorkloadProblem = components['schemas']['WorkloadProblem'];
export type IssueType = components['schemas']['IssueType'];
export type Rollout = components['schemas']['Rollout'];
export type RolloutStatus = components['schemas']['RolloutStatus'];

export async function searchKubernetes(req: KubeSearchRequest): Promise<KubeSearchPage> {
  if (!live) return mock.searchKubernetes(req);
  return post('/api/v1/orchestration/search', req);
}

export async function getKubeResource(uid: string): Promise<KubeDetail> {
  if (!live) return mock.getKubeResource(uid);
  return get('/api/v1/orchestration/resources/{uid}', { uid });
}

export async function listKubeClusters(): Promise<KubeClusterSummary[]> {
  if (!live) return mock.listKubeClusters();
  return get('/api/v1/kubernetes/clusters', {});
}

export async function getAutoscaling(): Promise<AutoscalingOverview> {
  if (!live) return autoscalingMock.getAutoscaling();
  return get('/api/v1/kubernetes/autoscaling', {}) as Promise<unknown> as Promise<AutoscalingOverview>;
}

export async function updateAutoscalingSettings(settings: AutoscalingSettings): Promise<AutoscalingSettings> {
  if (!live) return autoscalingMock.updateAutoscalingSettings(settings);
  return put('/api/v1/kubernetes/autoscaling/settings', {}, settings);
}

export async function createAutoscalingProfile(input: ProfileInput): Promise<SavedProfile> {
  if (!live) return autoscalingMock.createAutoscalingProfile(input);
  return post('/api/v1/kubernetes/autoscaling/profiles', input as never) as Promise<unknown> as Promise<SavedProfile>;
}

export async function updateAutoscalingProfile(id: string, input: ProfileInput): Promise<SavedProfile> {
  if (!live) return autoscalingMock.updateAutoscalingProfile(id, input);
  return put('/api/v1/kubernetes/autoscaling/profiles/{id}', { id }, input as never) as Promise<unknown> as Promise<SavedProfile>;
}

export async function deleteAutoscalingProfile(id: string): Promise<void> {
  if (!live) return autoscalingMock.deleteAutoscalingProfile(id);
  return del('/api/v1/kubernetes/autoscaling/profiles/{id}', { id });
}

export async function listWorkloadProblems(): Promise<WorkloadProblem[]> {
  if (!live) return autoscalingMock.listWorkloadProblems();
  return get('/api/v1/kubernetes/remediation', {});
}

export async function listRollouts(): Promise<Rollout[]> {
  if (!live) return autoscalingMock.listRollouts();
  return get('/api/v1/kubernetes/rollouts', {});
}
