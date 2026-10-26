// Cloud Network Monitoring data access. In "live" mode flows come from the Rust API (system-probe
// connections from the Datadog Agent, in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/network/flows   networkFlows   (flows between client and server groups, series, facets)
//   POST /api/v1/network/dns     networkDns     (DNS requests from clients to DNS servers)
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/network';

const live = API_MODE === 'live';

export type Flow = components['schemas']['Flow'];
export type FlowPage = components['schemas']['FlowPage'];
export type FlowRequest = components['schemas']['FlowRequest'];
export type NetworkPoint = components['schemas']['NetworkPoint'];
export type NetworkFacet = components['schemas']['NetworkFacet'];
export type DnsFlow = components['schemas']['DnsFlow'];
export type DnsPage = components['schemas']['DnsPage'];
export type DnsRequest = components['schemas']['DnsRequest'];
export type DnsPoint = components['schemas']['DnsPoint'];

export async function networkFlows(req: FlowRequest): Promise<FlowPage> {
  if (!live) return mock.networkFlows(req);
  return post('/api/v1/network/flows', req);
}

export async function networkDns(req: DnsRequest): Promise<DnsPage> {
  if (!live) return mock.networkDns(req);
  return post('/api/v1/network/dns', req);
}
