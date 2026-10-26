// Cloud Network Monitoring data access. In "live" mode flows come from the Rust API (system-probe
// connections from the Datadog Agent, in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/network/flows   networkFlows   (flows between client and server groups, series, facets)
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/network';

const live = API_MODE === 'live';

export type Flow = components['schemas']['Flow'];
export type FlowPage = components['schemas']['FlowPage'];
export type FlowRequest = components['schemas']['FlowRequest'];
export type NetworkPoint = components['schemas']['NetworkPoint'];
export type NetworkFacet = components['schemas']['NetworkFacet'];

export async function networkFlows(req: FlowRequest): Promise<FlowPage> {
  if (!live) return mock.networkFlows(req);
  return post('/api/v1/network/flows', req);
}
