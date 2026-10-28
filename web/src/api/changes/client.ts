// Infrastructure → Changes data access. In "live" mode changes come from the Rust API (Kubernetes
// rollouts, spec generations and crash events, APM versions, CI deployments, change events and the
// audit trail); otherwise the in-browser mock answers.
//   POST /api/v1/infrastructure/changes   searchChanges
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/changes';

const live = API_MODE === 'live';

export type Change = components['schemas']['Change'];
export type ChangeType = components['schemas']['ChangeType'];
export type ChangeSource = components['schemas']['ChangeSource'];
export type ChangeSearchRequest = components['schemas']['ChangeSearchRequest'];
export type ChangePage = components['schemas']['ChangePage'];
export type ChangeBucket = components['schemas']['ChangeBucket'];

export async function searchChanges(req: ChangeSearchRequest): Promise<ChangePage> {
  if (!live) return mock.searchChanges(req);
  return post('/api/v1/infrastructure/changes', req);
}
