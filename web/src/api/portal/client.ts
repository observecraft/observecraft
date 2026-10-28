// Developer Portal → Home and Settings. In "live" mode they come from the Rust API; otherwise the
// in-browser mock answers (mock/portal.ts).
//   GET    /api/v1/portal/connectors                  getPortalConnectors
//   PUT    /api/v1/portal/connectors/:provider        savePortalConnector (admins)
//   DELETE /api/v1/portal/connectors/:provider        deletePortalConnector (admins)
//   POST   /api/v1/portal/connectors/:provider/test   testPortalConnector (admins)
//   GET    /api/v1/portal/identities                  getPortalIdentities
//   PUT    /api/v1/portal/identities                  savePortalIdentities
//   GET    /api/v1/portal/home/prs?provider=          getPortalPullRequests
//   GET    /api/v1/portal/home/tickets?provider=      getPortalTickets
//   GET    /api/v1/portal/home/entities               getPortalEntities
//   PUT    /api/v1/portal/favorites/:entity           setPortalFavorite
//   DELETE /api/v1/portal/favorites/:entity           setPortalFavorite(…, false)
//   GET    /api/v1/portal/apps                        getPortalApps
//   POST   /api/v1/portal/apps                        createPortalApp
//   PUT    /api/v1/portal/apps/:id                    updatePortalApp
//   DELETE /api/v1/portal/apps/:id                    deletePortalApp
//   GET    /api/v1/portal/sources                     getPortalSources
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/portal';
import type {
  CodeHost,
  PortalApp,
  PortalAppInput,
  PortalConnectionTest,
  PortalConnector,
  PortalConnectorInput,
  PortalEntity,
  PortalIdentitiesInput,
  PortalIdentity,
  PortalProvider,
  PortalPullRequests,
  PortalSources,
  PortalTickets,
  TicketSource,
} from './types';

export * from './types';

const live = API_MODE === 'live';

export const getPortalConnectors = (): Promise<PortalConnector[]> => (live ? get('/api/v1/portal/connectors', {}) : mock.getConnectors());
export const savePortalConnector = (provider: PortalProvider, input: PortalConnectorInput): Promise<PortalConnector> =>
  live ? put('/api/v1/portal/connectors/{provider}', { provider }, input) : mock.saveConnector(provider, input);
export const deletePortalConnector = (provider: PortalProvider): Promise<void> => (live ? del('/api/v1/portal/connectors/{provider}', { provider }) : mock.deleteConnector(provider));
export const testPortalConnector = (provider: PortalProvider): Promise<PortalConnectionTest> =>
  live ? postTo('/api/v1/portal/connectors/{provider}/test', { provider }, undefined) : mock.testConnector(provider);
export const getPortalIdentities = (): Promise<PortalIdentity[]> => (live ? get('/api/v1/portal/identities', {}) : mock.getIdentities());
export const savePortalIdentities = (input: PortalIdentitiesInput): Promise<PortalIdentity[]> => (live ? put('/api/v1/portal/identities', {}, input) : mock.saveIdentities(input));
export const getPortalPullRequests = (provider: CodeHost, signal?: AbortSignal): Promise<PortalPullRequests> =>
  live ? get('/api/v1/portal/home/prs', {}, signal, { provider }) : mock.getPullRequests(provider);
export const getPortalTickets = (provider: TicketSource, signal?: AbortSignal): Promise<PortalTickets> =>
  live ? get('/api/v1/portal/home/tickets', {}, signal, { provider }) : mock.getTickets(provider);
export const getPortalEntities = (signal?: AbortSignal): Promise<PortalEntity[]> => (live ? get('/api/v1/portal/home/entities', {}, signal) : mock.getEntities());
export const setPortalFavorite = (entity: string, favorite: boolean): Promise<void> =>
  live ? (favorite ? put('/api/v1/portal/favorites/{entity}', { entity }, undefined as never) : del('/api/v1/portal/favorites/{entity}', { entity })) : mock.setFavorite(entity, favorite);
export const getPortalApps = (): Promise<PortalApp[]> => (live ? get('/api/v1/portal/apps', {}) : mock.getApps());
export const createPortalApp = (input: PortalAppInput): Promise<PortalApp[]> => (live ? post('/api/v1/portal/apps', input) : mock.createApp(input));
export const updatePortalApp = (id: string, input: PortalAppInput): Promise<PortalApp[]> => (live ? put('/api/v1/portal/apps/{id}', { id }, input) : mock.updateApp(id, input));
export const deletePortalApp = (id: string): Promise<void> => (live ? del('/api/v1/portal/apps/{id}', { id }) : mock.deleteApp(id));
export const getPortalSources = (): Promise<PortalSources> => (live ? get('/api/v1/portal/sources', {}) : mock.getSources());
