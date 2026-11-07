// Integrations → NetBird: the connection the server polls a NetBird account's management API with; its audit events
// arrive as logs (`source:netbird`), its peers as `netbird.peer.*` metrics.
//   GET    /api/v1/integrations/netbird        netbirdStatus
//   PUT    /api/v1/integrations/netbird        saveNetbird (admins)
//   DELETE /api/v1/integrations/netbird        deleteNetbird (admins)
//   POST   /api/v1/integrations/netbird/test   testNetbird (admins)
//   POST   /api/v1/integrations/netbird/sync   syncNetbird (admins)
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/netbird';

type S = components['schemas'];
export type NetbirdInput = S['NetbirdInput'];
export type NetbirdStatus = S['NetbirdStatus'];
export type NetbirdTest = S['NetbirdTest'];

export async function netbirdStatus(): Promise<NetbirdStatus> {
  if (API_MODE !== 'live') return mock.netbirdStatus();
  return get('/api/v1/integrations/netbird', {});
}

export async function saveNetbird(input: NetbirdInput): Promise<NetbirdStatus> {
  if (API_MODE !== 'live') return mock.saveNetbird(input);
  return put('/api/v1/integrations/netbird', {}, input);
}

export async function deleteNetbird(): Promise<void> {
  if (API_MODE !== 'live') return mock.deleteNetbird();
  return del('/api/v1/integrations/netbird', {});
}

export async function testNetbird(input: NetbirdInput): Promise<NetbirdTest> {
  if (API_MODE !== 'live') return mock.testNetbird(input);
  return post('/api/v1/integrations/netbird/test', input);
}

export async function syncNetbird(): Promise<void> {
  if (API_MODE !== 'live') return mock.syncNetbird();
  await postTo('/api/v1/integrations/netbird/sync', {}, undefined);
}
