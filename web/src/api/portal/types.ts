import type { components } from '../generated/schema';

type S = components['schemas'];
export type PortalProvider = S['PortalProvider'];
export type PortalConnector = S['PortalConnector'];
export type PortalConnectorInput = S['PortalConnectorInput'];
export type PortalConnectionTest = S['PortalConnectionTest'];
export type PortalIdentity = S['PortalIdentity'];
export type PortalIdentitiesInput = S['PortalIdentitiesInput'];
export type PortalListStatus = S['PortalListStatus'];
export type PortalPullRequests = S['PortalPullRequests'];
export type PortalPullRequest = S['PortalPullRequest'];
export type PortalPerson = S['PortalPerson'];
export type PrGroup = S['PrGroup'];
export type PortalTickets = S['PortalTickets'];
export type PortalTicket = S['PortalTicket'];
export type PortalEntity = S['PortalEntity'];
export type PortalApp = S['PortalApp'];
export type PortalAppInput = S['PortalAppInput'];
export type PortalSignal = S['PortalSignal'];
export type PortalSources = S['PortalSources'];
/** Ticket lists of the home page: the two trackers and our own incidents. */
export type TicketSource = 'jira' | 'linear' | 'work_items';
export type CodeHost = 'github' | 'gitlab';
