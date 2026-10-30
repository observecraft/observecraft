// Infrastructure → Resources → Changes: changes of the resources' configuration (created, deleted,
// fields changed), detected by the server's change job. Outside live mode the mock answers.
//   POST /api/v1/infrastructure/resource-changes   searchResourceChanges
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/resourceChanges';

const live = API_MODE === 'live';

export type ResourceChange = components['schemas']['ResourceChange'];
export type ResourceChangePage = components['schemas']['ResourceChangePage'];
export type ResourceChangeSearchRequest = components['schemas']['ResourceChangeSearchRequest'];

export async function searchResourceChanges(req: ResourceChangeSearchRequest, signal?: AbortSignal): Promise<ResourceChangePage> {
  if (!live) return mock.searchResourceChanges(req);
  return post('/api/v1/infrastructure/resource-changes', req, signal);
}
