// Integrations → FleetDM: the connection the server polls a Fleet server with; its activities arrive as logs
// (`source:fleetdm`), its hosts, host counts and policies as `fleetdm.*` metrics.
//   GET    /api/v1/integrations/fleetdm        fleetdmStatus
//   PUT    /api/v1/integrations/fleetdm        saveFleetdm (admins)
//   DELETE /api/v1/integrations/fleetdm        deleteFleetdm (admins)
//   POST   /api/v1/integrations/fleetdm/test   testFleetdm (admins)
//   POST   /api/v1/integrations/fleetdm/sync   syncFleetdm (admins)
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/fleetdm';

type S = components['schemas'];
export type FleetdmInput = S['FleetdmInput'];
export type FleetdmStatus = S['FleetdmStatus'];
export type FleetdmTest = S['FleetdmTest'];

export async function fleetdmStatus(): Promise<FleetdmStatus> {
  if (API_MODE !== 'live') return mock.fleetdmStatus();
  return get('/api/v1/integrations/fleetdm', {});
}

export async function saveFleetdm(input: FleetdmInput): Promise<FleetdmStatus> {
  if (API_MODE !== 'live') return mock.saveFleetdm(input);
  return put('/api/v1/integrations/fleetdm', {}, input);
}

export async function deleteFleetdm(): Promise<void> {
  if (API_MODE !== 'live') return mock.deleteFleetdm();
  return del('/api/v1/integrations/fleetdm', {});
}

export async function testFleetdm(input: FleetdmInput): Promise<FleetdmTest> {
  if (API_MODE !== 'live') return mock.testFleetdm(input);
  return post('/api/v1/integrations/fleetdm/test', input);
}

export async function syncFleetdm(): Promise<void> {
  if (API_MODE !== 'live') return mock.syncFleetdm();
  await postTo('/api/v1/integrations/fleetdm/sync', {}, undefined);
}
