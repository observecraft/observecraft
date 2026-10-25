// Kubernetes Explorer data access. In "live" mode objects come from the Rust API (the cluster
// collector's objects, events and kubelet metrics in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/orchestration/search          searchKubernetes
//   GET  /api/v1/orchestration/resources/:uid  getKubeResource
import type { components } from '../generated/schema';
import { API_MODE, get, post } from '../http';
import * as mock from '../mock/kubernetes';

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

export async function searchKubernetes(req: KubeSearchRequest): Promise<KubeSearchPage> {
  if (!live) return mock.searchKubernetes(req);
  return post('/api/v1/orchestration/search', req);
}

export async function getKubeResource(uid: string): Promise<KubeDetail> {
  if (!live) return mock.getKubeResource(uid);
  return get('/api/v1/orchestration/resources/{uid}', { uid });
}
