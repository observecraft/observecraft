// Integrations → Vanta: the private Vanta app the server sends the org's compliance evidence to, every hour.
//   GET    /api/v1/integrations/vanta        vantaStatus
//   PUT    /api/v1/integrations/vanta        saveVanta (admins)
//   DELETE /api/v1/integrations/vanta        deleteVanta (admins)
//   POST   /api/v1/integrations/vanta/test   testVanta (admins)
//   POST   /api/v1/integrations/vanta/sync   syncVanta (admins)
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/vanta';

type S = components['schemas'];
export type VantaInput = S['VantaInput'];
export type VantaStatus = S['VantaStatus'];
export type VantaTest = S['VantaTest'];
export type VantaTypeStatus = S['VantaTypeStatus'];

export async function vantaStatus(): Promise<VantaStatus> {
  if (API_MODE !== 'live') return mock.vantaStatus();
  return get('/api/v1/integrations/vanta', {});
}

export async function saveVanta(input: VantaInput): Promise<VantaStatus> {
  if (API_MODE !== 'live') return mock.saveVanta(input);
  return put('/api/v1/integrations/vanta', {}, input);
}

export async function deleteVanta(): Promise<void> {
  if (API_MODE !== 'live') return mock.deleteVanta();
  return del('/api/v1/integrations/vanta', {});
}

export async function testVanta(input: VantaInput): Promise<VantaTest> {
  if (API_MODE !== 'live') return mock.testVanta(input);
  return post('/api/v1/integrations/vanta/test', input);
}

export async function syncVanta(): Promise<void> {
  if (API_MODE !== 'live') return mock.syncVanta();
  await postTo('/api/v1/integrations/vanta/sync', {}, undefined);
}
