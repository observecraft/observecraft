// Integrations → connectors (docs/connectors.md): the catalog of connector packages and native connectors, and an
// org's connections of packages, which the server polls into logs, metrics, events and costs.
//   GET    /api/v1/connectors                                     listConnectors
//   GET    /api/v1/connectors/{id}                                getConnector (with the org's connections)
//   POST   /api/v1/connectors/{id}/connections                    createConnection (admins)
//   PUT    /api/v1/connectors/{id}/connections/{connection}       updateConnection (admins)
//   DELETE /api/v1/connectors/{id}/connections/{connection}       deleteConnection (admins)
//   POST   /api/v1/connectors/{id}/connections/{connection}/sync  syncConnection (admins)
//   POST   /api/v1/connectors/{id}/test                           testConnection (admins)
//   POST   /api/v1/connectors/{id}/push                           createPush (admins, push-only packages)
import type { components } from '../generated/schema';
import { API_MODE, del, get, postTo, put } from '../http';
import * as mock from '../mock/connectors';

type S = components['schemas'];
export type ConnectorCatalog = S['ConnectorCatalog'];
export type ConnectorEntry = S['ConnectorEntry'];
export type ConnectorField = S['ConnectorField'];
export type ConnectorDetail = S['ConnectorDetail'];
export type ConnectionView = S['ConnectionView'];
export type ConnectionInput = S['ConnectionInput'];
export type ConnectorTest = S['ConnectorTest'];
export type StreamStatus = S['StreamStatus'];

export async function listConnectors(): Promise<ConnectorCatalog> {
  if (API_MODE !== 'live') return mock.listConnectors();
  return get('/api/v1/connectors', {});
}

export async function getConnector(id: string): Promise<ConnectorDetail> {
  if (API_MODE !== 'live') return mock.getConnector(id);
  return get('/api/v1/connectors/{id}', { id });
}

export async function createConnection(id: string, input: ConnectionInput): Promise<ConnectionView> {
  if (API_MODE !== 'live') return mock.createConnection(id, input);
  return postTo('/api/v1/connectors/{id}/connections', { id }, input);
}

export async function updateConnection(id: string, connection: string, input: ConnectionInput): Promise<ConnectionView> {
  if (API_MODE !== 'live') return mock.updateConnection(id, connection, input);
  return put('/api/v1/connectors/{id}/connections/{connection}', { id, connection }, input);
}

export async function deleteConnection(id: string, connection: string): Promise<void> {
  if (API_MODE !== 'live') return mock.deleteConnection(id, connection);
  return del('/api/v1/connectors/{id}/connections/{connection}', { id, connection });
}

export async function syncConnection(id: string, connection: string): Promise<void> {
  if (API_MODE !== 'live') return mock.syncConnection(id, connection);
  await postTo('/api/v1/connectors/{id}/connections/{connection}/sync', { id, connection }, undefined);
}

export async function testConnection(id: string, input: ConnectionInput): Promise<ConnectorTest> {
  if (API_MODE !== 'live') return mock.testConnection(id, input);
  return postTo('/api/v1/connectors/{id}/test', { id }, input);
}

export async function createPush(id: string, input: ConnectionInput): Promise<ConnectorTest> {
  if (API_MODE !== 'live') return mock.createPush(id, input);
  return postTo('/api/v1/connectors/{id}/push', { id }, input);
}

/** The connector page of a connector: its own page (native ones), else the generic one. */
export function connectorPage(c: Pick<ConnectorEntry, 'id' | 'page'>): string {
  return c.page ?? `/integrations/connectors/${c.id}`;
}
