// Infrastructure → Resources → Cloudcraft: the architecture diagram of the resource catalog. In "live"
// mode it comes from the Rust API; otherwise the in-browser mock answers (mock/cloudMap.ts).
//   POST /api/v1/infrastructure/cloud-map   getCloudMap
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/cloudMap';

type S = components['schemas'];
export type CloudMap = S['CloudMap'];
export type CloudMapRequest = S['CloudMapRequest'];
export type CloudMapGroup = S['CloudMapGroup'];
export type CloudMapNode = S['CloudMapNode'];
export type CloudMapEdge = S['CloudMapEdge'];
export type CloudMapHealth = S['CloudMapHealth'];
export type CloudMapOverlay = S['CloudMapOverlay'];
export type CloudMapProvider = S['CloudMapProvider'];
export type CloudMapProviderSummary = S['CloudMapProviderSummary'];

/** Value of the group that merges a level's smallest groups. */
export const OTHER_GROUP = '__other__';

export async function getCloudMap(req: CloudMapRequest, signal?: AbortSignal): Promise<CloudMap> {
  if (API_MODE !== 'live') return mock.getCloudMap(req);
  return post('/api/v1/infrastructure/cloud-map', req, signal);
}
