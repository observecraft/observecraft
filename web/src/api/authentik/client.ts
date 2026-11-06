// Integrations → authentik: the connection the server polls an authentik instance's events with; the events
// arrive as logs (`source:authentik`).
//   GET    /api/v1/integrations/authentik        authentikStatus
//   PUT    /api/v1/integrations/authentik        saveAuthentik (admins)
//   DELETE /api/v1/integrations/authentik        deleteAuthentik (admins)
//   POST   /api/v1/integrations/authentik/test   testAuthentik (admins)
//   POST   /api/v1/integrations/authentik/sync   syncAuthentik (admins)
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/authentik';

type S = components['schemas'];
export type AuthentikInput = S['AuthentikInput'];
export type AuthentikStatus = S['AuthentikStatus'];
export type AuthentikTest = S['AuthentikTest'];

export async function authentikStatus(): Promise<AuthentikStatus> {
  if (API_MODE !== 'live') return mock.authentikStatus();
  return get('/api/v1/integrations/authentik', {});
}

export async function saveAuthentik(input: AuthentikInput): Promise<AuthentikStatus> {
  if (API_MODE !== 'live') return mock.saveAuthentik(input);
  return put('/api/v1/integrations/authentik', {}, input);
}

export async function deleteAuthentik(): Promise<void> {
  if (API_MODE !== 'live') return mock.deleteAuthentik();
  return del('/api/v1/integrations/authentik', {});
}

export async function testAuthentik(input: AuthentikInput): Promise<AuthentikTest> {
  if (API_MODE !== 'live') return mock.testAuthentik(input);
  return post('/api/v1/integrations/authentik/test', input);
}

export async function syncAuthentik(): Promise<void> {
  if (API_MODE !== 'live') return mock.syncAuthentik();
  await postTo('/api/v1/integrations/authentik/sync', {}, undefined);
}
